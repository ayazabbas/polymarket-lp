use anyhow::{Context, Result};
use futures::StreamExt;
use polymarket_client_sdk::auth;
use polymarket_client_sdk::clob::ws;
use polymarket_client_sdk::types::{B256, U256};
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// Events from the WebSocket feed relevant to the quoting engine.
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// New midpoint value for a token.
    MidpointUpdate { asset_id: String, midpoint: Decimal },
    /// Order book update with best bid/ask.
    BookUpdate {
        asset_id: String,
        best_bid: Option<Decimal>,
        best_ask: Option<Decimal>,
    },
    /// A fill event on one of our orders.
    OrderFill {
        order_id: String,
        size: Decimal,
        price: Decimal,
    },
    /// Connection lost, falling back to REST.
    Disconnected,
    /// Connection restored.
    Reconnected,
}

/// Manages WebSocket subscriptions and feeds events to the engine.
pub struct WsManager {
    event_tx: mpsc::Sender<WsEvent>,
    shutdown_tx: watch::Sender<bool>,
}

impl WsManager {
    /// Start WebSocket subscriptions for the given assets.
    /// Returns the manager and a receiver for events.
    pub async fn start(
        token_ids: Vec<String>,
        market_condition_id: Option<String>,
        credentials: Option<(auth::Credentials, polymarket_client_sdk::types::Address)>,
    ) -> Result<(Self, mpsc::Receiver<WsEvent>)> {
        let (event_tx, event_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let asset_ids: Vec<U256> = token_ids
            .iter()
            .filter_map(|id| U256::from_str(id).ok())
            .collect();

        // Spawn the market data subscription task
        let tx = event_tx.clone();
        let ids = asset_ids.clone();
        let mut rx = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                if *rx.borrow() {
                    break;
                }
                if let Err(e) = run_market_subscription(&tx, &ids, &mut rx).await {
                    warn!(error = %e, "Market WS subscription error, reconnecting...");
                    let _ = tx.send(WsEvent::Disconnected).await;
                    // Exponential backoff up to 30s
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    let _ = tx.send(WsEvent::Reconnected).await;
                }
            }
        });

        // Spawn user event subscription if authenticated
        if let Some((creds, address)) = credentials {
            if let Some(cond_id) = market_condition_id {
                let tx = event_tx.clone();
                let mut rx = shutdown_rx.clone();
                tokio::spawn(async move {
                    loop {
                        if *rx.borrow() {
                            break;
                        }
                        if let Err(e) =
                            run_user_subscription(&tx, &creds, address, &cond_id, &mut rx).await
                        {
                            warn!(error = %e, "User WS subscription error, reconnecting...");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                });
            }
        }

        Ok((
            Self {
                event_tx,
                shutdown_tx,
            },
            event_rx,
        ))
    }

    /// Shutdown all WebSocket connections.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

async fn run_market_subscription(
    tx: &mpsc::Sender<WsEvent>,
    asset_ids: &[U256],
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let ws_client = ws::Client::default();

    // Subscribe to midpoint updates
    let stream = ws_client
        .subscribe_midpoints(asset_ids.to_vec())
        .context("subscribing to midpoints")?;
    let mut stream = Box::pin(stream);

    info!(assets = asset_ids.len(), "WebSocket market subscription started");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            item = stream.next() => {
                match item {
                    Some(Ok(update)) => {
                        debug!(
                            asset_id = %update.asset_id,
                            midpoint = %update.midpoint,
                            "WS midpoint update"
                        );
                        let _ = tx.send(WsEvent::MidpointUpdate {
                            asset_id: update.asset_id.to_string(),
                            midpoint: update.midpoint,
                        }).await;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "WS stream error");
                        return Err(e.into());
                    }
                    None => {
                        info!("WS stream ended");
                        return Ok(());
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_user_subscription(
    tx: &mpsc::Sender<WsEvent>,
    credentials: &auth::Credentials,
    address: polymarket_client_sdk::types::Address,
    market_condition_id: &str,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let ws_client = ws::Client::default();
    let ws_auth = ws_client
        .authenticate(credentials.clone(), address)
        .context("authenticating WS client")?;

    let market_id =
        B256::from_str(market_condition_id).context("parsing market condition ID for WS")?;

    let stream = ws_auth
        .subscribe_trades(vec![market_id])
        .context("subscribing to user trades")?;
    let mut stream = Box::pin(stream);

    info!("WebSocket user subscription started");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            item = stream.next() => {
                match item {
                    Some(Ok(trade)) => {
                        info!(
                            side = ?trade.side,
                            size = %trade.size,
                            price = %trade.price,
                            "WS trade fill"
                        );
                        let _ = tx.send(WsEvent::OrderFill {
                            order_id: trade.taker_order_id.clone().unwrap_or_default(),
                            size: trade.size,
                            price: trade.price,
                        }).await;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "User WS stream error");
                        return Err(e.into());
                    }
                    None => {
                        info!("User WS stream ended");
                        return Ok(());
                    }
                }
            }
        }
    }

    Ok(())
}
