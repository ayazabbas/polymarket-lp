use anyhow::{Context, Result};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::MidpointRequest;
use polymarket_client_sdk::types::U256;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::config::StrategyConfig;
use crate::quoter::{self, Quote, QuoteParams};
use crate::scanner::MarketInfo;

/// State for a single market's quoting engine.
pub struct QuoteEngine {
    pub market: MarketInfo,
    pub config: StrategyConfig,
    pub dry_run: bool,
    pub last_midpoint: Option<Decimal>,
    pub last_requote: Option<Instant>,
    pub current_quotes: Vec<Quote>,
    /// Active order IDs on the exchange (empty in dry-run)
    pub active_order_ids: Vec<String>,
    /// Net inventory: positive = long YES tokens
    pub inventory_yes: Decimal,
    pub inventory_no: Decimal,
}

impl QuoteEngine {
    pub fn new(market: MarketInfo, config: StrategyConfig, dry_run: bool) -> Self {
        Self {
            market,
            config,
            dry_run,
            last_midpoint: None,
            last_requote: None,
            current_quotes: Vec::new(),
            active_order_ids: Vec::new(),
            inventory_yes: Decimal::ZERO,
            inventory_no: Decimal::ZERO,
        }
    }

    /// Fetch the current midpoint from the CLOB API.
    pub async fn fetch_midpoint(
        &self,
        clob_client: &clob::Client<impl polymarket_client_sdk::auth::state::State>,
    ) -> Result<Decimal> {
        let token_id = U256::from_str(&self.market.token_yes_id)
            .context("parsing YES token ID")?;
        let req = MidpointRequest::builder().token_id(token_id).build();
        let resp = clob_client
            .midpoint(&req)
            .await
            .context("fetching midpoint")?;
        Ok(resp.mid)
    }

    /// Determine if we should requote based on midpoint shift or timer.
    pub fn should_requote(&self, new_midpoint: Decimal) -> bool {
        let threshold = self.config.requote_threshold_cents / dec!(100);

        // Midpoint shift trigger
        if let Some(last_mid) = self.last_midpoint {
            if (new_midpoint - last_mid).abs() > threshold {
                debug!(
                    old_mid = %last_mid,
                    new_mid = %new_midpoint,
                    threshold = %threshold,
                    "Midpoint shift exceeds threshold"
                );
                return true;
            }
        } else {
            return true; // First quote
        }

        // Timer trigger
        if let Some(last_time) = self.last_requote {
            if last_time.elapsed() > Duration::from_secs(self.config.requote_interval_secs) {
                debug!("Requote timer expired");
                return true;
            }
        }

        false
    }

    /// Generate new quotes based on current midpoint.
    pub fn compute_quotes(&self, midpoint: Decimal) -> Vec<Quote> {
        let tick_size = Decimal::from_str(&self.market.tick_size).unwrap_or(dec!(0.01));

        // Calculate inventory skew factor
        let net_inventory = self.inventory_yes - self.inventory_no;
        let cap = self.config.inventory_cap;
        let skew = if cap > Decimal::ZERO {
            (net_inventory / cap).min(dec!(0.5)).max(dec!(-0.5))
        } else {
            Decimal::ZERO
        };

        let params = QuoteParams {
            midpoint,
            base_offset_cents: self.config.base_offset_cents,
            min_offset_cents: self.config.min_offset_cents,
            tick_size,
            order_size: self.config.order_size,
            num_levels: self.config.num_levels,
            fee_rate_bps: self.market.fee_rate_bps.map(|v| v as u32),
            max_incentive_spread: self.market.rewards_max_spread,
            min_incentive_size: self.market.rewards_min_size,
            inventory_skew: skew,
        };

        let quotes = quoter::generate_quotes(&params);

        // Log score estimates
        for q in &quotes {
            let bid_score = quoter::estimate_score(
                midpoint,
                q.bid_price,
                q.size,
                self.market.rewards_max_spread,
                self.market.rewards_min_size,
            );
            let ask_score = quoter::estimate_score(
                midpoint,
                q.ask_price,
                q.size,
                self.market.rewards_max_spread,
                self.market.rewards_min_size,
            );
            let total = quoter::two_sided_score(bid_score, ask_score);
            debug!(
                level = q.level,
                bid = %q.bid_price,
                ask = %q.ask_price,
                size = %q.size,
                bid_score = %bid_score,
                ask_score = %ask_score,
                total_score = %total,
                "Quote computed"
            );
        }

        quotes
    }

    /// Run one tick of the quoting engine: fetch midpoint, decide to requote, generate quotes.
    /// In dry-run mode, just logs the computed orders.
    pub async fn tick(
        &mut self,
        clob_client: &clob::Client<impl polymarket_client_sdk::auth::state::State>,
    ) -> Result<()> {
        let midpoint = self.fetch_midpoint(clob_client).await?;

        if !self.should_requote(midpoint) {
            return Ok(());
        }

        let quotes = self.compute_quotes(midpoint);

        if self.dry_run {
            self.log_dry_run(&quotes, midpoint);
        } else {
            // Phase 2 will implement actual order placement
            warn!("Live order placement not yet implemented");
        }

        self.last_midpoint = Some(midpoint);
        self.last_requote = Some(Instant::now());
        self.current_quotes = quotes;

        Ok(())
    }

    fn log_dry_run(&self, quotes: &[Quote], midpoint: Decimal) {
        info!(
            market = %self.market.question,
            midpoint = %midpoint,
            num_quotes = quotes.len(),
            "[DRY-RUN] Quoting"
        );
        for q in quotes {
            info!(
                level = q.level,
                bid = %q.bid_price,
                ask = %q.ask_price,
                size = %q.size,
                spread = %(q.ask_price - q.bid_price),
                "[DRY-RUN] Quote"
            );
        }
    }
}
