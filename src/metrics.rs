use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// Tracks PnL, fill rates, and other metrics for a single market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMetrics {
    pub condition_id: String,
    pub question: String,
    pub spread_pnl: Decimal,
    pub reward_pnl: Decimal,
    pub rebate_pnl: Decimal,
    pub total_fills: u64,
    pub total_orders: u64,
    pub uptime_ticks: u64,
    pub total_ticks: u64,
    pub inventory_yes: Decimal,
    pub inventory_no: Decimal,
    pub last_midpoint: Option<Decimal>,
    pub start_time: DateTime<Utc>,
    pub last_update: DateTime<Utc>,
}

impl MarketMetrics {
    pub fn new(condition_id: String, question: String) -> Self {
        let now = Utc::now();
        Self {
            condition_id,
            question,
            spread_pnl: Decimal::ZERO,
            reward_pnl: Decimal::ZERO,
            rebate_pnl: Decimal::ZERO,
            total_fills: 0,
            total_orders: 0,
            uptime_ticks: 0,
            total_ticks: 0,
            inventory_yes: Decimal::ZERO,
            inventory_no: Decimal::ZERO,
            last_midpoint: None,
            start_time: now,
            last_update: now,
        }
    }

    pub fn fill_rate(&self) -> Decimal {
        if self.total_orders == 0 {
            return Decimal::ZERO;
        }
        Decimal::new(self.total_fills as i64, 0) / Decimal::new(self.total_orders as i64, 0)
    }

    pub fn uptime_pct(&self) -> Decimal {
        if self.total_ticks == 0 {
            return Decimal::ZERO;
        }
        Decimal::new(self.uptime_ticks as i64, 0) / Decimal::new(self.total_ticks as i64, 0)
            * dec!(100)
    }

    pub fn total_pnl(&self) -> Decimal {
        self.spread_pnl + self.reward_pnl + self.rebate_pnl
    }

    pub fn record_tick(&mut self, had_orders: bool) {
        self.total_ticks += 1;
        if had_orders {
            self.uptime_ticks += 1;
        }
        self.last_update = Utc::now();
    }

    pub fn record_fill(&mut self, spread_capture: Decimal) {
        self.total_fills += 1;
        self.spread_pnl += spread_capture;
    }

    pub fn record_orders(&mut self, count: u64) {
        self.total_orders += count;
    }

    pub fn record_reward(&mut self, amount: Decimal) {
        self.reward_pnl += amount;
    }

    pub fn record_rebate(&mut self, amount: Decimal) {
        self.rebate_pnl += amount;
    }
}

/// Aggregate metrics across all markets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioMetrics {
    pub markets: HashMap<String, MarketMetrics>,
    pub daily_rewards: Vec<DailyReward>,
    pub session_start: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReward {
    pub date: String,
    pub amount: Decimal,
    pub expected: Decimal,
}

impl PortfolioMetrics {
    pub fn new() -> Self {
        Self {
            markets: HashMap::new(),
            daily_rewards: Vec::new(),
            session_start: Utc::now(),
        }
    }

    pub fn total_pnl(&self) -> Decimal {
        self.markets.values().map(|m| m.total_pnl()).sum()
    }

    pub fn total_spread_pnl(&self) -> Decimal {
        self.markets.values().map(|m| m.spread_pnl).sum()
    }

    pub fn total_reward_pnl(&self) -> Decimal {
        self.markets.values().map(|m| m.reward_pnl).sum()
    }

    pub fn total_fills(&self) -> u64 {
        self.markets.values().map(|m| m.total_fills).sum()
    }

    pub fn avg_fill_rate(&self) -> Decimal {
        let rates: Vec<Decimal> = self
            .markets
            .values()
            .filter(|m| m.total_orders > 0)
            .map(|m| m.fill_rate())
            .collect();
        if rates.is_empty() {
            return Decimal::ZERO;
        }
        let sum: Decimal = rates.iter().sum();
        sum / Decimal::new(rates.len() as i64, 0)
    }

    pub fn avg_uptime(&self) -> Decimal {
        let uptimes: Vec<Decimal> = self
            .markets
            .values()
            .filter(|m| m.total_ticks > 0)
            .map(|m| m.uptime_pct())
            .collect();
        if uptimes.is_empty() {
            return Decimal::ZERO;
        }
        let sum: Decimal = uptimes.iter().sum();
        sum / Decimal::new(uptimes.len() as i64, 0)
    }

    /// Save metrics to a JSON file for persistence.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("serializing metrics")?;
        std::fs::write(path, json)
            .context("writing metrics file")?;
        info!(path = ?path, "Metrics saved");
        Ok(())
    }

    /// Load metrics from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .context("reading metrics file")?;
        let metrics: Self = serde_json::from_str(&contents)
            .context("parsing metrics file")?;
        Ok(metrics)
    }
}

/// Send a Telegram alert message.
pub async fn send_telegram_alert(
    bot_token: &str,
    chat_id: &str,
    message: &str,
) -> Result<()> {
    if bot_token.is_empty() || chat_id.is_empty() {
        return Ok(());
    }

    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        bot_token
    );

    let client = reqwest::Client::new();
    client
        .post(&url)
        .form(&[("chat_id", chat_id), ("text", message)])
        .send()
        .await
        .context("sending Telegram alert")?;

    info!(message, "Telegram alert sent");
    Ok(())
}

/// Format a status dashboard string for the CLI.
pub fn format_dashboard(
    portfolio: &PortfolioMetrics,
    market_engines: &[(String, Decimal, Decimal, usize)], // (question, midpoint, inventory, open_orders)
) -> String {
    let mut out = String::new();
    out.push_str("=== Polymarket LP Bot Status ===\n\n");

    out.push_str(&format!(
        "Session start: {}\n",
        portfolio.session_start.format("%Y-%m-%d %H:%M UTC")
    ));
    out.push_str(&format!("Total PnL:     ${:.4}\n", portfolio.total_pnl()));
    out.push_str(&format!(
        "  Spread:      ${:.4}\n",
        portfolio.total_spread_pnl()
    ));
    out.push_str(&format!(
        "  Rewards:     ${:.4}\n",
        portfolio.total_reward_pnl()
    ));
    out.push_str(&format!("Total fills:   {}\n", portfolio.total_fills()));
    out.push_str(&format!(
        "Avg fill rate: {:.1}%\n",
        portfolio.avg_fill_rate() * dec!(100)
    ));
    out.push_str(&format!(
        "Avg uptime:    {:.1}%\n",
        portfolio.avg_uptime()
    ));

    out.push_str("\n--- Markets ---\n");
    out.push_str(&format!(
        "{:<40} {:>8} {:>10} {:>8}\n",
        "Question", "Midpoint", "Inventory", "Orders"
    ));
    out.push_str(&"-".repeat(70));
    out.push('\n');

    for (question, midpoint, inventory, orders) in market_engines {
        let q = if question.len() > 38 {
            format!("{}...", &question[..35])
        } else {
            question.clone()
        };
        out.push_str(&format!(
            "{:<40} {:>8.4} {:>10.1} {:>8}\n",
            q, midpoint, inventory, orders
        ));
    }

    if !portfolio.daily_rewards.is_empty() {
        out.push_str("\n--- Recent Rewards ---\n");
        for reward in portfolio.daily_rewards.iter().rev().take(7) {
            out.push_str(&format!(
                "  {} — ${:.2} (expected: ${:.2})\n",
                reward.date, reward.amount, reward.expected
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_metrics_fill_rate() {
        let mut m = MarketMetrics::new("test".into(), "Test?".into());
        m.total_orders = 100;
        m.total_fills = 25;
        assert_eq!(m.fill_rate(), dec!(0.25));
    }

    #[test]
    fn test_market_metrics_uptime() {
        let mut m = MarketMetrics::new("test".into(), "Test?".into());
        for _ in 0..80 {
            m.record_tick(true);
        }
        for _ in 0..20 {
            m.record_tick(false);
        }
        assert_eq!(m.uptime_pct(), dec!(80));
    }

    #[test]
    fn test_portfolio_total_pnl() {
        let mut p = PortfolioMetrics::new();
        let mut m1 = MarketMetrics::new("a".into(), "Q1".into());
        m1.spread_pnl = dec!(10);
        m1.reward_pnl = dec!(5);
        let mut m2 = MarketMetrics::new("b".into(), "Q2".into());
        m2.spread_pnl = dec!(3);
        m2.reward_pnl = dec!(2);
        m2.rebate_pnl = dec!(1);
        p.markets.insert("a".into(), m1);
        p.markets.insert("b".into(), m2);
        assert_eq!(p.total_pnl(), dec!(21));
    }

    #[test]
    fn test_metrics_save_load() {
        let mut p = PortfolioMetrics::new();
        let m = MarketMetrics::new("test".into(), "Question?".into());
        p.markets.insert("test".into(), m);

        let path = std::env::temp_dir().join("polymarket_lp_test_metrics.json");
        p.save(&path).unwrap();
        let loaded = PortfolioMetrics::load(&path).unwrap();
        assert_eq!(loaded.markets.len(), 1);
        std::fs::remove_file(&path).ok();
    }
}
