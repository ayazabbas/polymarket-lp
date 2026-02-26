use anyhow::{Context, Result};
use polymarket_client_sdk::gamma;
use polymarket_client_sdk::gamma::types::request::MarketsRequest;
use rust_decimal::Decimal;
use tracing::info;

/// Processed market info relevant for LP decisions.
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub condition_id: String,
    pub question: String,
    pub token_yes_id: String,
    pub token_no_id: String,
    pub active: bool,
    pub closed: bool,
    pub liquidity: Decimal,
    pub volume: Decimal,
    pub reward_daily_estimate: Decimal,
    pub fee_rate_bps: Option<i32>,
    pub tick_size: String,
    pub rewards_min_size: Option<Decimal>,
    pub rewards_max_spread: Option<Decimal>,
    /// Higher = better opportunity (reward / existing liquidity)
    pub score: Decimal,
}

/// Fetch all active markets from Gamma API and extract LP-relevant info.
pub async fn scan_markets(gamma_client: &gamma::Client) -> Result<Vec<MarketInfo>> {
    info!("Scanning active markets via Gamma API...");

    let request = MarketsRequest::builder()
        .closed(false)
        .limit(100)
        .build();

    let markets = gamma_client
        .markets(&request)
        .await
        .context("fetching markets from Gamma API")?;

    info!(count = markets.len(), "Fetched markets from Gamma");

    let mut results = Vec::new();
    for market in &markets {
        let condition_id = match &market.condition_id {
            Some(id) => id.to_string(),
            None => continue,
        };

        let question = market
            .question
            .clone()
            .unwrap_or_else(|| "Unknown".into());

        let active = market.active.unwrap_or(false);
        let closed = market.closed.unwrap_or(true);
        if !active || closed {
            continue;
        }

        // Extract token IDs
        let tokens = match &market.clob_token_ids {
            Some(ids) if ids.len() >= 2 => ids.clone(),
            _ => continue,
        };

        let liquidity = market.liquidity.unwrap_or(Decimal::ZERO);
        let volume = market.volume.unwrap_or(Decimal::ZERO);

        // Use competitive field as a proxy for reward attractiveness
        let reward_daily = market.competitive.unwrap_or(Decimal::ZERO);

        let tick_size = market
            .order_price_min_tick_size
            .map(|d| d.to_string())
            .unwrap_or_else(|| "0.01".into());

        let rewards_min_size = market.rewards_min_size;
        let rewards_max_spread = market.rewards_max_spread;

        let fee_rate_bps = market.taker_base_fee;

        // Score: reward / liquidity ratio (higher = less competition per reward dollar)
        let score = if liquidity > Decimal::ZERO {
            reward_daily / liquidity * Decimal::new(10000, 0)
        } else if reward_daily > Decimal::ZERO {
            Decimal::new(99999, 0)
        } else {
            Decimal::ZERO
        };

        results.push(MarketInfo {
            condition_id,
            question,
            token_yes_id: tokens[0].to_string(),
            token_no_id: tokens[1].to_string(),
            active,
            closed,
            liquidity,
            volume,
            reward_daily_estimate: reward_daily,
            fee_rate_bps,
            tick_size,
            rewards_min_size,
            rewards_max_spread,
            score,
        });
    }

    // Sort by score descending
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    info!(eligible = results.len(), "Market scan complete");

    Ok(results)
}

/// Rank markets and filter by minimum daily reward threshold.
pub fn rank_markets(markets: &[MarketInfo], min_daily_reward: Decimal, max_count: usize) -> Vec<MarketInfo> {
    markets
        .iter()
        .filter(|m| m.reward_daily_estimate >= min_daily_reward)
        .take(max_count)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rank_markets_filters_by_reward() {
        let markets = vec![
            make_test_market("A", Decimal::new(10, 0), Decimal::new(1000, 0)),
            make_test_market("B", Decimal::new(2, 0), Decimal::new(500, 0)),
            make_test_market("C", Decimal::new(20, 0), Decimal::new(1000, 0)),
        ];
        // Pre-sort by score descending (as scan_markets does)
        let mut markets = markets;
        markets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let ranked = rank_markets(&markets, Decimal::new(5, 0), 10);
        assert_eq!(ranked.len(), 2); // A=10, C=20 pass; B=2 fails
        assert_eq!(ranked[0].question, "C"); // C has higher score (200 vs 100)
    }

    #[test]
    fn test_rank_markets_respects_max_count() {
        let markets = vec![
            make_test_market("A", Decimal::new(100, 0), Decimal::new(1000, 0)),
            make_test_market("B", Decimal::new(50, 0), Decimal::new(1000, 0)),
            make_test_market("C", Decimal::new(30, 0), Decimal::new(1000, 0)),
        ];
        let ranked = rank_markets(&markets, Decimal::ZERO, 2);
        assert_eq!(ranked.len(), 2);
    }

    fn make_test_market(question: &str, reward: Decimal, liquidity: Decimal) -> MarketInfo {
        let score = if liquidity > Decimal::ZERO {
            reward / liquidity * Decimal::new(10000, 0)
        } else {
            Decimal::ZERO
        };
        MarketInfo {
            condition_id: format!("cond_{question}"),
            question: question.into(),
            token_yes_id: "token_yes".into(),
            token_no_id: "token_no".into(),
            active: true,
            closed: false,
            liquidity,
            volume: Decimal::new(10000, 0),
            reward_daily_estimate: reward,
            fee_rate_bps: None,
            tick_size: "0.01".into(),
            rewards_min_size: None,
            rewards_max_spread: None,
            score,
        }
    }
}
