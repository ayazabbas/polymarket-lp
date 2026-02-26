mod client;
mod config;
mod engine;
mod quoter;
mod scanner;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL, Table};
use rust_decimal::Decimal;
use std::path::PathBuf;
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
        Commands::Run { live, market } => {
            cmd_run(&config, live, market).await?;
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

async fn cmd_run(config: &config::Config, live: bool, market: Option<String>) -> Result<()> {
    let dry_run = !live;
    if dry_run {
        info!("DRY-RUN mode (use --live to place real orders)");
    }

    // For dry-run, we can use unauthenticated client; for live we need auth
    let clob_client = if live {
        warn!("Live mode: authenticating...");
        // TODO: return authenticated client for Phase 2
        bail!("Live order placement requires Phase 2 implementation");
    } else {
        client::create_unauthenticated_client()?
    };

    // Find the target market
    let gamma_client = client::create_gamma_client()?;
    let markets = scanner::scan_markets(&gamma_client).await?;

    let target = if let Some(ref cond_id) = market {
        markets
            .iter()
            .find(|m| m.condition_id.starts_with(cond_id))
            .cloned()
    } else {
        // Pick the top-ranked market
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

    let mut engine = engine::QuoteEngine::new(target, config.strategy.clone(), dry_run);

    info!("Starting quoting loop (Ctrl+C to stop)...");

    let tick_interval = std::time::Duration::from_secs(config.strategy.requote_interval_secs);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Shutdown signal received");
                break;
            }
            result = engine.tick(&clob_client) => {
                if let Err(e) = result {
                    warn!(error = %e, "Engine tick error");
                }
            }
        }

        tokio::time::sleep(tick_interval).await;
    }

    info!("Quoting engine stopped");
    Ok(())
}

async fn cmd_status(_config: &config::Config) -> Result<()> {
    println!("Status dashboard will be implemented in Phase 6");
    Ok(())
}
