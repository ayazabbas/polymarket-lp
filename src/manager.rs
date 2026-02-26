use anyhow::{Context, Result};
use polymarket_client_sdk::auth;
use polymarket_client_sdk::auth::Signer;
use polymarket_client_sdk::clob;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::Config;
use crate::engine::QuoteEngine;
use crate::orders;
use crate::risk::{self, MarketInventory};
use crate::scanner::{self, MarketInfo};

/// Rate limiter to stay within Polymarket's API limits.
pub struct RateLimiter {
    /// Timestamps of recent order submissions
    order_timestamps: Vec<Instant>,
    /// Max orders per 10s burst
    burst_limit: usize,
    /// Max orders per 10min sustained
    sustained_limit: usize,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            order_timestamps: Vec::new(),
            burst_limit: 3500,
            sustained_limit: 36000,
        }
    }

    /// Check if we can place `count` orders right now.
    pub fn can_place(&mut self, count: usize) -> bool {
        let now = Instant::now();
        // Clean old timestamps
        self.order_timestamps
            .retain(|t| now.duration_since(*t) < Duration::from_secs(600));

        let burst_window = Duration::from_secs(10);
        let burst_count = self
            .order_timestamps
            .iter()
            .filter(|t| now.duration_since(**t) < burst_window)
            .count();

        if burst_count + count > self.burst_limit {
            warn!(
                current = burst_count,
                requested = count,
                "Rate limit: burst limit would be exceeded"
            );
            return false;
        }

        if self.order_timestamps.len() + count > self.sustained_limit {
            warn!(
                current = self.order_timestamps.len(),
                requested = count,
                "Rate limit: sustained limit would be exceeded"
            );
            return false;
        }

        true
    }

    /// Record that `count` orders were placed.
    pub fn record(&mut self, count: usize) {
        let now = Instant::now();
        for _ in 0..count {
            self.order_timestamps.push(now);
        }
    }
}

/// Manages multiple QuoteEngines across markets.
pub struct MarketManager {
    pub engines: HashMap<String, QuoteEngine>,
    pub config: Config,
    pub rate_limiter: RateLimiter,
    pub last_rescan: Instant,
    pub rescan_interval: Duration,
    pub capital_allocations: HashMap<String, Decimal>,
}

impl MarketManager {
    pub fn new(config: Config) -> Self {
        Self {
            engines: HashMap::new(),
            config,
            rate_limiter: RateLimiter::new(),
            last_rescan: Instant::now(),
            rescan_interval: Duration::from_secs(3600), // Rescan hourly
            capital_allocations: HashMap::new(),
        }
    }

    /// Initialize engines for the given markets with capital allocation.
    pub fn initialize_markets(&mut self, markets: Vec<MarketInfo>) {
        // Calculate capital allocation
        let scores: Vec<(String, Decimal)> = markets
            .iter()
            .map(|m| (m.condition_id.clone(), m.score))
            .collect();

        self.capital_allocations = risk::allocate_capital(
            &scores,
            self.config.risk.max_total_capital,
            self.config.risk.max_per_market,
        )
        .into_iter()
        .collect();

        for market in markets {
            let cond_id = market.condition_id.clone();
            if self.engines.contains_key(&cond_id) {
                continue;
            }

            let allocation = self
                .capital_allocations
                .get(&cond_id)
                .copied()
                .unwrap_or(Decimal::ZERO);

            // Adjust order size based on allocation
            let mut strategy = self.config.strategy.clone();
            if allocation > Decimal::ZERO {
                // Scale order size proportionally to allocation
                let base_capital = self.config.risk.max_per_market;
                if base_capital > Decimal::ZERO {
                    let scale = allocation / base_capital;
                    strategy.order_size = (strategy.order_size * scale).round();
                    strategy.order_size = strategy.order_size.max(Decimal::ONE);
                }
            }

            info!(
                market = %market.question,
                allocation = %allocation,
                order_size = %strategy.order_size,
                "Adding market to manager"
            );

            let engine = QuoteEngine::new(market, strategy, false);
            self.engines.insert(cond_id, engine);
        }

        info!(total_markets = self.engines.len(), "Markets initialized");
    }

    /// Remove markets that are no longer rewarded or have been resolved.
    pub fn remove_stale_markets(&mut self, active_ids: &[String]) {
        let stale: Vec<String> = self
            .engines
            .keys()
            .filter(|id| !active_ids.contains(id))
            .cloned()
            .collect();

        for id in &stale {
            info!(condition_id = %id, "Removing stale market");
            self.engines.remove(id);
        }
    }

    /// Check if hourly rescan is due.
    pub fn needs_rescan(&self) -> bool {
        self.last_rescan.elapsed() > self.rescan_interval
    }

    /// Perform a rescan: fetch fresh markets, add new ones, remove stale ones.
    pub async fn rescan(
        &mut self,
        gamma_client: &polymarket_client_sdk::gamma::Client,
    ) -> Result<()> {
        info!("Rescanning markets...");

        let all_markets = scanner::scan_markets(gamma_client).await?;
        let ranked = scanner::rank_markets(
            &all_markets,
            self.config.markets.min_reward_daily,
            self.config.markets.max_markets,
        );

        let active_ids: Vec<String> = ranked.iter().map(|m| m.condition_id.clone()).collect();

        // Add new markets
        let new_markets: Vec<MarketInfo> = ranked
            .into_iter()
            .filter(|m| !self.engines.contains_key(&m.condition_id))
            .collect();

        if !new_markets.is_empty() {
            info!(count = new_markets.len(), "New markets discovered");
            self.initialize_markets(new_markets);
        }

        // Remove stale
        self.remove_stale_markets(&active_ids);

        // Check for sponsored markets (high reward/competition)
        for (_, engine) in &self.engines {
            if engine.market.reward_daily_estimate > dec!(50) {
                info!(
                    market = %engine.market.question,
                    reward = %engine.market.reward_daily_estimate,
                    "Sponsored market detected — high reward opportunity"
                );
            }
        }

        self.last_rescan = Instant::now();
        info!(total_markets = self.engines.len(), "Rescan complete");
        Ok(())
    }

    /// Run one tick across all managed markets with rate limiting.
    pub async fn tick_all(
        &mut self,
        clob_client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
        signer: &impl Signer,
    ) -> Result<()> {
        // Check kill switch across all markets
        let inventories: Vec<(&str, MarketInventory, Decimal)> = self
            .engines
            .values()
            .map(|e| {
                let inv = MarketInventory {
                    yes_tokens: e.inventory_yes,
                    no_tokens: e.inventory_no,
                    total_bought_value: e.total_bought_value,
                    total_sold_value: e.total_sold_value,
                };
                let mid = e.last_midpoint.unwrap_or(dec!(0.5));
                (e.market.question.as_str(), inv, mid)
            })
            .collect();

        let inv_refs: Vec<(&str, &MarketInventory, Decimal)> = inventories
            .iter()
            .map(|(name, inv, mid)| (*name, inv, *mid))
            .collect();

        if risk::should_kill_switch(&inv_refs, &self.config.risk) {
            warn!("Kill switch activated — cancelling all orders");
            self.cancel_all_markets(clob_client).await?;
            return Ok(());
        }

        // Tick each engine, respecting rate limits
        let condition_ids: Vec<String> = self.engines.keys().cloned().collect();
        for cond_id in condition_ids {
            let engine = match self.engines.get_mut(&cond_id) {
                Some(e) => e,
                None => continue,
            };

            // Estimate orders needed for this tick (4 per level * num_levels)
            let estimated_orders = (engine.config.num_levels * 4) as usize;
            if !self.rate_limiter.can_place(estimated_orders) {
                warn!(
                    market = %engine.market.question,
                    "Skipping tick due to rate limit"
                );
                continue;
            }

            match engine.tick_live(clob_client, signer).await {
                Ok(()) => {
                    let actual_orders = engine.tracked_orders.len();
                    self.rate_limiter.record(actual_orders);
                }
                Err(e) => {
                    warn!(
                        market = %engine.market.question,
                        error = %e,
                        "Engine tick failed"
                    );
                }
            }
        }

        Ok(())
    }

    /// Cancel all orders across all markets.
    pub async fn cancel_all_markets(
        &mut self,
        clob_client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    ) -> Result<()> {
        // Use the bulk cancel endpoint for efficiency
        orders::cancel_all(clob_client).await?;

        // Clear local state
        for engine in self.engines.values_mut() {
            engine.tracked_orders.clear();
        }

        info!("All orders across all markets cancelled");
        Ok(())
    }

    /// Get aggregate portfolio stats.
    pub fn portfolio_stats(&self) -> PortfolioStats {
        let mut total_capital = Decimal::ZERO;
        let mut total_yes = Decimal::ZERO;
        let mut total_no = Decimal::ZERO;
        let mut total_pnl = Decimal::ZERO;
        let mut active_markets = 0;

        for engine in self.engines.values() {
            total_yes += engine.inventory_yes;
            total_no += engine.inventory_no;
            total_capital += engine.total_bought_value - engine.total_sold_value;

            if let Some(mid) = engine.last_midpoint {
                let inv = MarketInventory {
                    yes_tokens: engine.inventory_yes,
                    no_tokens: engine.inventory_no,
                    total_bought_value: engine.total_bought_value,
                    total_sold_value: engine.total_sold_value,
                };
                total_pnl += inv.unrealized_pnl(mid);
            }

            if !engine.tracked_orders.is_empty() {
                active_markets += 1;
            }
        }

        PortfolioStats {
            total_markets: self.engines.len(),
            active_markets,
            total_capital_deployed: total_capital,
            total_yes_tokens: total_yes,
            total_no_tokens: total_no,
            total_unrealized_pnl: total_pnl,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortfolioStats {
    pub total_markets: usize,
    pub active_markets: usize,
    pub total_capital_deployed: Decimal,
    pub total_yes_tokens: Decimal,
    pub total_no_tokens: Decimal,
    pub total_unrealized_pnl: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_basic() {
        let mut limiter = RateLimiter::new();
        assert!(limiter.can_place(100));
        limiter.record(100);
        assert!(limiter.can_place(100));
    }

    #[test]
    fn test_rate_limiter_burst_limit() {
        let mut limiter = RateLimiter::new();
        limiter.burst_limit = 10;
        assert!(limiter.can_place(10));
        limiter.record(10);
        assert!(!limiter.can_place(1));
    }
}
