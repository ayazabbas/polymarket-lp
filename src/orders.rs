use anyhow::{Context, Result};
use polymarket_client_sdk::auth;
use polymarket_client_sdk::auth::Signer;
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::{OrderType, Side};
use polymarket_client_sdk::types::{Decimal, U256};
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::quoter::Quote;

/// Represents an order we've placed on the exchange.
#[derive(Debug, Clone)]
pub struct TrackedOrder {
    pub order_id: String,
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub filled: Decimal,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
}

/// Place a batch of limit orders for a market.
pub async fn place_quotes(
    client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    signer: &impl Signer,
    token_yes_id: &str,
    token_no_id: &str,
    quotes: &[Quote],
) -> Result<Vec<TrackedOrder>> {
    let yes_id = U256::from_str(token_yes_id).context("parsing YES token ID")?;
    let no_id = U256::from_str(token_no_id).context("parsing NO token ID")?;

    let mut signed_orders = Vec::new();
    let mut order_metadata = Vec::new();

    for quote in quotes {
        // YES token BID (buying YES)
        let yes_bid = client
            .limit_order()
            .token_id(yes_id)
            .side(Side::Buy)
            .price(quote.bid_price)
            .size(quote.size)
            .order_type(OrderType::GTC)
            .build()
            .await
            .context("building YES bid order")?;
        let signed = client.sign(signer, yes_bid).await.context("signing YES bid")?;
        order_metadata.push((token_yes_id.to_string(), Side::Buy, quote.bid_price, quote.size));
        signed_orders.push(signed);

        // YES token ASK (selling YES)
        let yes_ask = client
            .limit_order()
            .token_id(yes_id)
            .side(Side::Sell)
            .price(quote.ask_price)
            .size(quote.size)
            .order_type(OrderType::GTC)
            .build()
            .await
            .context("building YES ask order")?;
        let signed = client.sign(signer, yes_ask).await.context("signing YES ask")?;
        order_metadata.push((token_yes_id.to_string(), Side::Sell, quote.ask_price, quote.size));
        signed_orders.push(signed);

        // NO token BID (complementary price)
        let no_bid_price = Decimal::ONE - quote.ask_price;
        if no_bid_price > Decimal::ZERO {
            let no_bid = client
                .limit_order()
                .token_id(no_id)
                .side(Side::Buy)
                .price(no_bid_price)
                .size(quote.size)
                .order_type(OrderType::GTC)
                .build()
                .await
                .context("building NO bid order")?;
            let signed = client.sign(signer, no_bid).await.context("signing NO bid")?;
            order_metadata.push((token_no_id.to_string(), Side::Buy, no_bid_price, quote.size));
            signed_orders.push(signed);
        }

        // NO token ASK (complementary price)
        let no_ask_price = Decimal::ONE - quote.bid_price;
        if no_ask_price < Decimal::ONE {
            let no_ask = client
                .limit_order()
                .token_id(no_id)
                .side(Side::Sell)
                .price(no_ask_price)
                .size(quote.size)
                .order_type(OrderType::GTC)
                .build()
                .await
                .context("building NO ask order")?;
            let signed = client.sign(signer, no_ask).await.context("signing NO ask")?;
            order_metadata.push((token_no_id.to_string(), Side::Sell, no_ask_price, quote.size));
            signed_orders.push(signed);
        }
    }

    if signed_orders.is_empty() {
        return Ok(vec![]);
    }

    // Batch post (up to 15 per call)
    let mut tracked = Vec::new();
    let mut meta_iter = order_metadata.into_iter();

    // Drain signed_orders into batches of 15
    let mut remaining = signed_orders;
    while !remaining.is_empty() {
        let batch: Vec<_> = remaining
            .drain(..remaining.len().min(15))
            .collect();
        let batch_size = batch.len();
        let batch_meta: Vec<_> = (&mut meta_iter).take(batch_size).collect();

        let responses = client
            .post_orders(batch)
            .await
            .context("posting order batch")?;

        for (resp, meta) in responses.iter().zip(batch_meta.iter()) {
            if resp.success {
                info!(
                    order_id = %resp.order_id,
                    side = ?meta.1,
                    price = %meta.2,
                    size = %meta.3,
                    "Order placed"
                );
                tracked.push(TrackedOrder {
                    order_id: resp.order_id.clone(),
                    token_id: meta.0.clone(),
                    side: meta.1.clone(),
                    price: meta.2,
                    size: meta.3,
                    filled: Decimal::ZERO,
                    status: OrderStatus::Open,
                });
            } else {
                warn!(
                    error = resp.error_msg.as_deref().unwrap_or("unknown"),
                    side = ?meta.1,
                    price = %meta.2,
                    "Order placement failed"
                );
            }
        }
    }

    debug!(count = tracked.len(), "Orders placed successfully");
    Ok(tracked)
}

/// Cancel a list of orders by ID.
pub async fn cancel_orders(
    client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    order_ids: &[String],
) -> Result<usize> {
    if order_ids.is_empty() {
        return Ok(0);
    }

    let id_refs: Vec<&str> = order_ids.iter().map(|s| s.as_str()).collect();
    let mut cancelled = 0;

    for chunk in id_refs.chunks(20) {
        let resp = client
            .cancel_orders(chunk)
            .await
            .context("cancelling orders")?;

        cancelled += resp.canceled.len();

        if !resp.not_canceled.is_empty() {
            debug!(
                count = resp.not_canceled.len(),
                "Some orders not cancelled (may already be filled)"
            );
        }
    }

    info!(cancelled, total = order_ids.len(), "Orders cancelled");
    Ok(cancelled)
}

/// Cancel all orders on the exchange.
pub async fn cancel_all(
    client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
) -> Result<()> {
    client
        .cancel_all_orders()
        .await
        .context("cancelling all orders")?;
    info!("All orders cancelled");
    Ok(())
}

/// Reconcile tracked orders with exchange state to detect fills.
pub async fn reconcile_orders(
    client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    tracked: &mut Vec<TrackedOrder>,
) -> Result<()> {
    for order in tracked.iter_mut() {
        if order.status == OrderStatus::Filled || order.status == OrderStatus::Cancelled {
            continue;
        }
        match client.order(&order.order_id).await {
            Ok(resp) => {
                let matched = resp.size_matched;
                let orig_size = resp.original_size;
                order.filled = matched;

                if matched >= orig_size {
                    order.status = OrderStatus::Filled;
                    info!(
                        order_id = %order.order_id,
                        side = ?order.side,
                        price = %order.price,
                        "Order fully filled"
                    );
                } else if matched > Decimal::ZERO {
                    order.status = OrderStatus::PartiallyFilled;
                }
            }
            Err(e) => {
                debug!(order_id = %order.order_id, error = %e, "Failed to fetch order status");
            }
        }
    }
    Ok(())
}
