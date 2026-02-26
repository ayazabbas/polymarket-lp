use anyhow::{Context, Result};
use polymarket_client_sdk::auth;
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::{AssetType, SignatureType};
use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
use rust_decimal::Decimal;
use tracing::{info, warn};

/// Check USDC balance and token balances for a given asset.
pub async fn check_balances(
    client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
) -> Result<BalanceInfo> {
    // Check collateral (USDC) balance
    let usdc_req = BalanceAllowanceRequest::builder()
        .asset_type(AssetType::Collateral)
        .signature_type(SignatureType::Eoa)
        .build();

    let usdc_resp = client
        .balance_allowance(usdc_req)
        .await
        .context("checking USDC balance")?;

    info!(
        balance = %usdc_resp.balance,
        "USDC balance"
    );

    Ok(BalanceInfo {
        usdc_balance: usdc_resp.balance,
    })
}

#[derive(Debug, Clone)]
pub struct BalanceInfo {
    pub usdc_balance: Decimal,
}

/// Split USDC into YES + NO token pairs.
/// This is done via the CTF contract on Polygon.
/// NOTE: The SDK's CTF feature handles the on-chain interaction.
/// For now, we log the intent and provide the interface.
pub async fn split_usdc_to_tokens(
    _client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    _condition_id: &str,
    amount: Decimal,
) -> Result<()> {
    // TODO: Implement via CTF relayer when SDK exposes the split method.
    // The relayer endpoint is rate-limited to 25 req/min.
    info!(
        amount = %amount,
        "Split USDC → YES + NO tokens (CTF operation)"
    );
    warn!("CTF split not yet implemented — requires relayer integration");
    Ok(())
}

/// Merge YES + NO token pairs back into USDC.
/// Useful to reduce exposure and free capital.
pub async fn merge_tokens_to_usdc(
    _client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    _condition_id: &str,
    amount: Decimal,
) -> Result<()> {
    // TODO: Implement via CTF relayer
    info!(
        amount = %amount,
        "Merge YES + NO → USDC (CTF operation)"
    );
    warn!("CTF merge not yet implemented — requires relayer integration");
    Ok(())
}

/// Redeem winning tokens after market resolution ($1 each).
pub async fn redeem_winning_tokens(
    _client: &clob::Client<auth::state::Authenticated<auth::Normal>>,
    _condition_id: &str,
    amount: Decimal,
) -> Result<()> {
    // TODO: Implement via CTF relayer
    info!(
        amount = %amount,
        "Redeem winning tokens (CTF operation)"
    );
    warn!("CTF redeem not yet implemented — requires relayer integration");
    Ok(())
}

/// Detect if a market has been resolved.
/// Returns the winning outcome index if resolved, None if still active.
pub fn check_resolution(market_closed: bool) -> Option<ResolutionResult> {
    if market_closed {
        Some(ResolutionResult { resolved: true })
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct ResolutionResult {
    pub resolved: bool,
}
