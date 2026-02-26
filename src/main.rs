mod client;
mod config;
mod engine;
mod inventory;
mod orders;
mod quoter;
mod risk;
mod scanner;
mod ws;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL, Table};
use polymarket_client_sdk::auth::{LocalSigner, Signer};
use polymarket_client_sdk::POLYGON;
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::str::FromStr;
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "polymarket-lp", about = "Polymarket liquidity provider bot")]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan and rank active rewarded markets
    Scan {
        /// Minimum daily reward to display ($)
        #[arg(short, long)]
        min_reward: Option<f64>,
        /// Maximum number of markets to show
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
    },
    /// Run the LP bot (dry-run by default)
    Run {
        /// Actually place orders (disable dry-run)
        #[arg(long)]
        live: bool,
        /// Target a specific market condition ID
        #[arg(short, long)]
        market: Option<String>,
        /// Disable WebSocket (use REST polling only)
        #[arg(long)]
        no_ws: bool,
    },
    /// Show current status, positions, and PnL
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = if cli.config.exists() {
        config::Config::load(&cli.config)?
    } else {
        config::Config {
            wallet: config::WalletConfig {
                private_key_env: "POLYMARKET_PRIVATE_KEY".into(),
                signature_type: "eoa".into(),
            },
            strategy: config::StrategyConfig::default(),
            markets: config::MarketsConfig::default(),
            risk: config::RiskConfig::default(),
            monitoring: config::MonitoringConfig::default(),
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.monitoring.log_level)),
        )
        .init();

    match cli.command {
        Commands::Scan { min_reward, limit } => {
            cmd_scan(&config, min_reward, limit).await?;
        }
        Commands::Run { live, market, no_ws } => {
            cmd_run(&config, live, market, no_ws).await?;
        }
        Commands::Status => {
            cmd_status(&config).await?;
        }
    }

    Ok(())
}

async fn cmd_scan(config: &config::Config, min_reward: Option<f64>, limit: usize) -> Result<()> {
    let gamma_client = client::create_gamma_client()?;
    let all_markets = scanner::scan_markets(&gamma_client).await?;

    let min_reward_dec = min_reward
        .map(|v| Decimal::try_from(v).unwrap_or(config.markets.min_reward_daily))
        .unwrap_or(config.markets.min_reward_daily);

    let ranked = scanner::rank_markets(&all_markets, min_reward_dec, limit);

    if ranked.is_empty() {
        println!("No markets found matching criteria (min_reward=${min_reward_dec}/day)");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        "#", "Question", "Daily Reward", "Liquidity", "Score", "Tick", "Condition ID",
    ]);

    for (i, m) in ranked.iter().enumerate() {
        let question = if m.question.len() > 50 {
            format!("{}...", &m.question[..47])
        } else {
            m.question.clone()
        };
        table.add_row(vec![
            format!("{}", i + 1),
            question,
            format!("${:.2}", m.reward_daily_estimate),
            format!("${:.0}", m.liquidity),
            format!("{:.1}", m.score),
            m.tick_size.clone(),
            m.condition_id[..12.min(m.condition_id.len())].to_string(),
        ]);
    }

    println!("{table}");
    println!(
        "\nFound {} rewarded markets (showing top {})",
        all_markets
            .iter()
            .filter(|m| m.reward_daily_estimate >= min_reward_dec)
            .count(),
        ranked.len()
    );

    Ok(())
}

async fn cmd_run(
    config: &config::Config,
    live: bool,
    market: Option<String>,
    no_ws: bool,
) -> Result<()> {
    let dry_run = !live;
    if dry_run {
        info!("DRY-RUN mode (use --live to place real orders)");
    }

    // Find the target market
    let gamma_client = client::create_gamma_client()?;
    let markets = scanner::scan_markets(&gamma_client).await?;

    let target = if let Some(ref cond_id) = market {
        markets
            .iter()
            .find(|m| m.condition_id.starts_with(cond_id))
            .cloned()
    } else {
        scanner::rank_markets(&markets, config.markets.min_reward_daily, 1)
            .into_iter()
            .next()
    };

    let target = match target {
        Some(m) => m,
        None => bail!("No suitable market found"),
    };

    info!(
        market = %target.question,
        condition_id = %target.condition_id,
        "Selected market"
    );

    let tick_interval = std::time::Duration::from_secs(config.strategy.requote_interval_secs);

    if live {
        let auth_client = client::create_authenticated_client(config).await?;
        let private_key = config.private_key()?;
        let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));

        let mut engine_inst =
            engine::QuoteEngine::new(target.clone(), config.strategy.clone(), false);

        // Start WebSocket if not disabled
        let ws_manager = if !no_ws {
            let token_ids = vec![target.token_yes_id.clone(), target.token_no_id.clone()];
            let creds = Some((
                auth_client.credentials().clone(),
                auth_client.address(),
            ));
            match ws::WsManager::start(token_ids, Some(target.condition_id.clone()), creds).await {
                Ok((mgr, rx)) => {
                    engine_inst.ws_connected = true;
                    info!("WebSocket connected");
                    Some((mgr, rx))
                }
                Err(e) => {
                    warn!(error = %e, "Failed to start WebSocket, falling back to REST");
                    None
                }
            }
        } else {
            None
        };

        info!("Starting LIVE quoting loop (Ctrl+C to stop)...");

        if let Some((mgr, mut ws_rx)) = ws_manager {
            // WS-driven loop: react to WS events, fallback to REST on disconnect
            loop {
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        info!("Shutdown signal received, cancelling all orders...");
                        mgr.shutdown();
                        if let Err(e) = engine_inst.cancel_all(&auth_client).await {
                            warn!(error = %e, "Error cancelling orders during shutdown");
                        }
                        break;
                    }
                    Some(event) = ws_rx.recv() => {
                        let should_requote = engine_inst.handle_ws_event(event);
                        if should_requote {
                            if let Some(mid) = engine_inst.last_midpoint {
                                let quotes = engine_inst.compute_quotes(mid);
                                // Cancel stale + place new
                                let stale: Vec<String> = engine_inst.tracked_orders.iter()
                                    .filter(|o| o.status == orders::OrderStatus::Open || o.status == orders::OrderStatus::PartiallyFilled)
                                    .map(|o| o.order_id.clone())
                                    .collect();
                                if !stale.is_empty() {
                                    let _ = orders::cancel_orders(&auth_client, &stale).await;
                                }
                                match orders::place_quotes(&auth_client, &signer, &engine_inst.market.token_yes_id, &engine_inst.market.token_no_id, &quotes).await {
                                    Ok(new_orders) => {
                                        engine_inst.tracked_orders = new_orders;
                                        engine_inst.current_quotes = quotes;
                                        engine_inst.last_requote = Some(std::time::Instant::now());
                                    }
                                    Err(e) => warn!(error = %e, "Failed to place orders"),
                                }
                            }
                        }
                    }
                    // Fallback REST tick when WS is disconnected
                    _ = tokio::time::sleep(tick_interval), if !engine_inst.ws_connected => {
                        if let Err(e) = engine_inst.tick_live(&auth_client, &signer).await {
                            warn!(error = %e, "REST fallback tick error");
                        }
                    }
                }
            }
        } else {
            // Pure REST loop (no WS)
            loop {
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        info!("Shutdown signal received, cancelling all orders...");
                        if let Err(e) = engine_inst.cancel_all(&auth_client).await {
                            warn!(error = %e, "Error cancelling orders during shutdown");
                        }
                        break;
                    }
                    result = engine_inst.tick_live(&auth_client, &signer) => {
                        if let Err(e) = result {
                            warn!(error = %e, "Engine tick error");
                        }
                    }
                }
                tokio::time::sleep(tick_interval).await;
            }
        }
    } else {
        // Dry-run mode with optional WS for midpoint
        let clob_client = client::create_unauthenticated_client()?;
        let mut engine_inst =
            engine::QuoteEngine::new(target.clone(), config.strategy.clone(), true);

        let ws_manager = if !no_ws {
            let token_ids = vec![target.token_yes_id.clone(), target.token_no_id.clone()];
            match ws::WsManager::start(token_ids, None, None).await {
                Ok((mgr, rx)) => {
                    engine_inst.ws_connected = true;
                    info!("WebSocket connected (dry-run)");
                    Some((mgr, rx))
                }
                Err(e) => {
                    warn!(error = %e, "Failed to start WebSocket, using REST polling");
                    None
                }
            }
        } else {
            None
        };

        info!("Starting DRY-RUN quoting loop (Ctrl+C to stop)...");

        if let Some((mgr, mut ws_rx)) = ws_manager {
            loop {
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        mgr.shutdown();
                        info!("Shutdown signal received");
                        break;
                    }
                    Some(event) = ws_rx.recv() => {
                        let should_requote = engine_inst.handle_ws_event(event);
                        if should_requote {
                            if let Some(mid) = engine_inst.last_midpoint {
                                let quotes = engine_inst.compute_quotes(mid);
                                engine_inst.log_dry_run_quotes(&quotes, mid);
                                engine_inst.current_quotes = quotes;
                                engine_inst.last_requote = Some(std::time::Instant::now());
                            }
                        }
                    }
                    _ = tokio::time::sleep(tick_interval), if !engine_inst.ws_connected => {
                        if let Err(e) = engine_inst.tick_dry_run(&clob_client).await {
                            warn!(error = %e, "REST fallback tick error");
                        }
                    }
                }
            }
        } else {
            loop {
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        info!("Shutdown signal received");
                        break;
                    }
                    result = engine_inst.tick_dry_run(&clob_client) => {
                        if let Err(e) = result {
                            warn!(error = %e, "Engine tick error");
                        }
                    }
                }
                tokio::time::sleep(tick_interval).await;
            }
        }
    }

    info!("Quoting engine stopped");
    Ok(())
}

async fn cmd_status(_config: &config::Config) -> Result<()> {
    println!("Status dashboard will be implemented in Phase 6");
    Ok(())
}
