use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub wallet: WalletConfig,
    #[serde(default)]
    pub strategy: StrategyConfig,
    #[serde(default)]
    pub markets: MarketsConfig,
    #[serde(default)]
    pub risk: RiskConfig,
    #[serde(default)]
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    #[serde(default = "default_private_key_env")]
    pub private_key_env: String,
    #[serde(default = "default_signature_type")]
    pub signature_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    #[serde(default = "default_base_offset")]
    pub base_offset_cents: Decimal,
    #[serde(default = "default_min_offset")]
    pub min_offset_cents: Decimal,
    #[serde(default = "default_requote_interval")]
    pub requote_interval_secs: u64,
    #[serde(default = "default_requote_threshold")]
    pub requote_threshold_cents: Decimal,
    #[serde(default = "default_order_size")]
    pub order_size: Decimal,
    #[serde(default = "default_num_levels")]
    pub num_levels: u32,
    #[serde(default = "default_inventory_cap")]
    pub inventory_cap: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketsConfig {
    #[serde(default = "default_market_mode")]
    pub mode: String,
    #[serde(default = "default_max_markets")]
    pub max_markets: usize,
    #[serde(default = "default_min_reward_daily")]
    pub min_reward_daily: Decimal,
    #[serde(default = "default_prefer_fee_enabled")]
    pub prefer_fee_enabled: bool,
    #[serde(default)]
    pub manual_markets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    #[serde(default = "default_max_total_capital")]
    pub max_total_capital: Decimal,
    #[serde(default = "default_max_per_market")]
    pub max_per_market: Decimal,
    #[serde(default = "default_kill_switch_loss")]
    pub kill_switch_loss: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub telegram_bot_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
}

// Defaults
fn default_private_key_env() -> String {
    "POLYMARKET_PRIVATE_KEY".into()
}
fn default_signature_type() -> String {
    "eoa".into()
}
fn default_base_offset() -> Decimal {
    Decimal::new(10, 1) // 1.0
}
fn default_min_offset() -> Decimal {
    Decimal::new(5, 1) // 0.5
}
fn default_requote_interval() -> u64 {
    30
}
fn default_requote_threshold() -> Decimal {
    Decimal::new(5, 1) // 0.5
}
fn default_order_size() -> Decimal {
    Decimal::new(500, 0)
}
fn default_num_levels() -> u32 {
    2
}
fn default_inventory_cap() -> Decimal {
    Decimal::new(5000, 0)
}
fn default_market_mode() -> String {
    "auto".into()
}
fn default_max_markets() -> usize {
    20
}
fn default_min_reward_daily() -> Decimal {
    Decimal::new(5, 0)
}
fn default_prefer_fee_enabled() -> bool {
    true
}
fn default_max_total_capital() -> Decimal {
    Decimal::new(2000, 0)
}
fn default_max_per_market() -> Decimal {
    Decimal::new(500, 0)
}
fn default_kill_switch_loss() -> Decimal {
    Decimal::new(100, 0)
}
fn default_log_level() -> String {
    "info".into()
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            base_offset_cents: default_base_offset(),
            min_offset_cents: default_min_offset(),
            requote_interval_secs: default_requote_interval(),
            requote_threshold_cents: default_requote_threshold(),
            order_size: default_order_size(),
            num_levels: default_num_levels(),
            inventory_cap: default_inventory_cap(),
        }
    }
}

impl Default for MarketsConfig {
    fn default() -> Self {
        Self {
            mode: default_market_mode(),
            max_markets: default_max_markets(),
            min_reward_daily: default_min_reward_daily(),
            prefer_fee_enabled: default_prefer_fee_enabled(),
            manual_markets: vec![],
        }
    }
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_total_capital: default_max_total_capital(),
            max_per_market: default_max_per_market(),
            kill_switch_loss: default_kill_switch_loss(),
        }
    }
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading config from {path:?}"))?;
        let config: Config =
            toml::from_str(&contents).with_context(|| format!("parsing config from {path:?}"))?;
        Ok(config)
    }

    pub fn private_key(&self) -> Result<String> {
        std::env::var(&self.wallet.private_key_env).with_context(|| {
            format!(
                "environment variable '{}' not set",
                self.wallet.private_key_env
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_roundtrip() {
        let config = Config {
            wallet: WalletConfig {
                private_key_env: "POLYMARKET_PRIVATE_KEY".into(),
                signature_type: "eoa".into(),
            },
            strategy: StrategyConfig::default(),
            markets: MarketsConfig::default(),
            risk: RiskConfig::default(),
            monitoring: MonitoringConfig::default(),
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.strategy.base_offset_cents, config.strategy.base_offset_cents);
        assert_eq!(parsed.markets.max_markets, 20);
    }

    #[test]
    fn test_minimal_config() {
        let toml_str = r#"
[wallet]
private_key_env = "MY_KEY"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.wallet.private_key_env, "MY_KEY");
        assert_eq!(config.strategy.order_size, Decimal::new(500, 0));
    }
}
