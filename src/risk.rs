use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{info, warn};

use crate::config::{RiskConfig, StrategyConfig};

/// Inventory state for a single market.
#[derive(Debug, Clone)]
pub struct MarketInventory {
    pub yes_tokens: Decimal,
    pub no_tokens: Decimal,
    pub total_bought_value: Decimal,
    pub total_sold_value: Decimal,
}

impl MarketInventory {
    pub fn new() -> Self {
        Self {
            yes_tokens: Decimal::ZERO,
            no_tokens: Decimal::ZERO,
            total_bought_value: Decimal::ZERO,
            total_sold_value: Decimal::ZERO,
        }
    }

    /// Net inventory: YES - NO. Positive = long YES.
    pub fn net_position(&self) -> Decimal {
        self.yes_tokens - self.no_tokens
    }

    /// Unrealized PnL at a given midpoint (approximate).
    pub fn unrealized_pnl(&self, midpoint: Decimal) -> Decimal {
        let yes_value = self.yes_tokens * midpoint;
        let no_value = self.no_tokens * (Decimal::ONE - midpoint);
        let mark_to_market = yes_value + no_value;
        mark_to_market + self.total_sold_value - self.total_bought_value
    }

    /// Total capital deployed (cost basis of current positions).
    pub fn capital_deployed(&self) -> Decimal {
        self.total_bought_value - self.total_sold_value
    }
}

/// Risk decision for quoting on a specific side.
#[derive(Debug, Clone, PartialEq)]
pub enum QuoteSideDecision {
    /// Quote normally
    Normal,
    /// Quote with adjusted offset (tighter or wider)
    Adjusted { offset_multiplier: Decimal },
    /// Do not quote this side
    Paused,
}

/// Determine whether to quote each side based on inventory state.
pub fn inventory_check(
    inventory: &MarketInventory,
    strategy: &StrategyConfig,
) -> (QuoteSideDecision, QuoteSideDecision) {
    let net = inventory.net_position();
    let cap = strategy.inventory_cap;

    if cap.is_zero() {
        return (QuoteSideDecision::Normal, QuoteSideDecision::Normal);
    }

    let ratio = net / cap;

    // Bid side (buying YES) and Ask side (selling YES)
    let bid_decision;
    let ask_decision;

    if ratio >= Decimal::ONE {
        // At or above YES cap: stop buying YES
        bid_decision = QuoteSideDecision::Paused;
        ask_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: dec!(0.5),
        };
        warn!(
            net_position = %net,
            cap = %cap,
            "YES inventory at cap, pausing bids"
        );
    } else if ratio <= -Decimal::ONE {
        // At or below NO cap: stop selling YES (buying NO)
        bid_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: dec!(0.5),
        };
        ask_decision = QuoteSideDecision::Paused;
        warn!(
            net_position = %net,
            cap = %cap,
            "NO inventory at cap, pausing asks"
        );
    } else if ratio > dec!(0.5) {
        // Approaching YES cap: widen bid, tighten ask
        let multiplier = Decimal::ONE + ratio;
        bid_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: multiplier,
        };
        ask_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: Decimal::ONE / multiplier,
        };
    } else if ratio < dec!(-0.5) {
        // Approaching NO cap: tighten bid, widen ask
        let multiplier = Decimal::ONE + ratio.abs();
        bid_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: Decimal::ONE / multiplier,
        };
        ask_decision = QuoteSideDecision::Adjusted {
            offset_multiplier: multiplier,
        };
    } else {
        bid_decision = QuoteSideDecision::Normal;
        ask_decision = QuoteSideDecision::Normal;
    }

    (bid_decision, ask_decision)
}

/// Check if the kill switch should be triggered based on total losses.
pub fn should_kill_switch(
    inventories: &[(&str, &MarketInventory, Decimal)], // (market_name, inventory, midpoint)
    risk_config: &RiskConfig,
) -> bool {
    let total_pnl: Decimal = inventories
        .iter()
        .map(|(_, inv, mid)| inv.unrealized_pnl(*mid))
        .sum();

    if total_pnl < -risk_config.kill_switch_loss {
        warn!(
            total_pnl = %total_pnl,
            threshold = %risk_config.kill_switch_loss,
            "KILL SWITCH triggered"
        );
        return true;
    }

    false
}

/// Calculate optimal capital allocation across markets.
/// Returns fraction of total capital to allocate to each market.
pub fn allocate_capital(
    market_scores: &[(String, Decimal)], // (market_id, reward_score)
    total_capital: Decimal,
    max_per_market: Decimal,
) -> Vec<(String, Decimal)> {
    if market_scores.is_empty() {
        return vec![];
    }

    let total_score: Decimal = market_scores.iter().map(|(_, s)| s).sum();
    if total_score.is_zero() {
        // Equal allocation
        let per_market = (total_capital / Decimal::new(market_scores.len() as i64, 0))
            .min(max_per_market);
        return market_scores
            .iter()
            .map(|(id, _)| (id.clone(), per_market))
            .collect();
    }

    market_scores
        .iter()
        .map(|(id, score)| {
            let fraction = *score / total_score;
            let allocation = (total_capital * fraction).min(max_per_market);
            info!(
                market = %id,
                score = %score,
                allocation = %allocation,
                "Capital allocation"
            );
            (id.clone(), allocation)
        })
        .collect()
}

/// Determine if holding tokens near resolution is worthwhile.
/// Near-resolution tokens (>0.90 or <0.10) earn ~4% APY equivalent.
pub fn holding_reward_factor(midpoint: Decimal, days_to_resolution: Option<u32>) -> Decimal {
    let days = days_to_resolution.unwrap_or(30);
    if days == 0 {
        return Decimal::ZERO;
    }

    // Only relevant for tokens likely to win (>0.85) or lose (<0.15)
    let confidence = if midpoint > dec!(0.85) {
        midpoint
    } else if midpoint < dec!(0.15) {
        Decimal::ONE - midpoint
    } else {
        return Decimal::ZERO;
    };

    // ~4% APY prorated for remaining days
    let annual_rate = dec!(0.04);
    let daily_rate = annual_rate / dec!(365);
    let holding_value = confidence * daily_rate * Decimal::new(days as i64, 0);

    holding_value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inventory_check_normal() {
        let inv = MarketInventory {
            yes_tokens: dec!(100),
            no_tokens: dec!(80),
            total_bought_value: Decimal::ZERO,
            total_sold_value: Decimal::ZERO,
        };
        let config = StrategyConfig {
            inventory_cap: dec!(5000),
            ..Default::default()
        };
        let (bid, ask) = inventory_check(&inv, &config);
        assert_eq!(bid, QuoteSideDecision::Normal);
        assert_eq!(ask, QuoteSideDecision::Normal);
    }

    #[test]
    fn test_inventory_check_at_cap() {
        let inv = MarketInventory {
            yes_tokens: dec!(5000),
            no_tokens: Decimal::ZERO,
            total_bought_value: Decimal::ZERO,
            total_sold_value: Decimal::ZERO,
        };
        let config = StrategyConfig {
            inventory_cap: dec!(5000),
            ..Default::default()
        };
        let (bid, ask) = inventory_check(&inv, &config);
        assert_eq!(bid, QuoteSideDecision::Paused);
        assert!(matches!(ask, QuoteSideDecision::Adjusted { .. }));
    }

    #[test]
    fn test_unrealized_pnl() {
        let inv = MarketInventory {
            yes_tokens: dec!(1000),
            no_tokens: Decimal::ZERO,
            total_bought_value: dec!(400), // bought 1000 YES at 0.40
            total_sold_value: Decimal::ZERO,
        };
        // Midpoint moved to 0.50, so YES worth 500
        let pnl = inv.unrealized_pnl(dec!(0.50));
        assert_eq!(pnl, dec!(100)); // 500 - 400
    }

    #[test]
    fn test_capital_allocation() {
        let scores = vec![
            ("market_a".into(), dec!(100)),
            ("market_b".into(), dec!(50)),
            ("market_c".into(), dec!(50)),
        ];
        let allocations = allocate_capital(&scores, dec!(2000), dec!(1000));
        assert_eq!(allocations.len(), 3);
        assert_eq!(allocations[0].1, dec!(1000)); // 50% of 2000 = 1000, capped at 1000
        assert_eq!(allocations[1].1, dec!(500)); // 25% of 2000
    }

    #[test]
    fn test_holding_reward_factor() {
        // High confidence near resolution
        let factor = holding_reward_factor(dec!(0.95), Some(7));
        assert!(factor > Decimal::ZERO);

        // Low confidence — no holding reward
        let factor = holding_reward_factor(dec!(0.50), Some(7));
        assert_eq!(factor, Decimal::ZERO);
    }

    #[test]
    fn test_kill_switch() {
        let inv = MarketInventory {
            yes_tokens: dec!(1000),
            no_tokens: Decimal::ZERO,
            total_bought_value: dec!(600),
            total_sold_value: Decimal::ZERO,
        };
        let risk = RiskConfig {
            kill_switch_loss: dec!(100),
            ..Default::default()
        };
        // Midpoint at 0.40: value = 400, PnL = 400 - 600 = -200
        assert!(should_kill_switch(
            &[("test", &inv, dec!(0.40))],
            &risk
        ));
        // Midpoint at 0.55: value = 550, PnL = 550 - 600 = -50 (within threshold)
        assert!(!should_kill_switch(
            &[("test", &inv, dec!(0.55))],
            &risk
        ));
    }
}
