use anyhow::{Context, Result};
use polymarket_client_sdk::auth::{LocalSigner, Signer};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::POLYGON;
use std::str::FromStr;
use tracing::info;

use crate::config::Config;

/// Create an unauthenticated CLOB client for read-only operations.
pub fn create_unauthenticated_client() -> Result<clob::Client<polymarket_client_sdk::auth::state::Unauthenticated>> {
    let client = clob::Client::new("https://clob.polymarket.com", clob::Config::default())
        .context("creating CLOB client")?;
    Ok(client)
}

/// Create an authenticated CLOB client from config.
pub async fn create_authenticated_client(
    config: &Config,
) -> Result<clob::Client<polymarket_client_sdk::auth::state::Authenticated<polymarket_client_sdk::auth::Normal>>> {
    let private_key = config.private_key()?;
    let signer = LocalSigner::from_str(&private_key)
        .context("parsing private key")?
        .with_chain_id(Some(POLYGON));

    let clob_config = clob::Config::builder()
        .use_server_time(true)
        .build();

    let unauth = clob::Client::new("https://clob.polymarket.com", clob_config)
        .context("creating CLOB client")?;

    let sig_type = match config.wallet.signature_type.as_str() {
        "proxy" => polymarket_client_sdk::clob::types::SignatureType::Proxy,
        "gnosis_safe" => polymarket_client_sdk::clob::types::SignatureType::GnosisSafe,
        _ => polymarket_client_sdk::clob::types::SignatureType::Eoa,
    };

    let client = unauth
        .authentication_builder(&signer)
        .signature_type(sig_type)
        .authenticate()
        .await
        .context("authenticating CLOB client")?;

    info!(address = %client.address(), "Authenticated with Polymarket CLOB");
    Ok(client)
}

/// Create a Gamma API client for market discovery.
pub fn create_gamma_client() -> Result<polymarket_client_sdk::gamma::Client> {
    let client = polymarket_client_sdk::gamma::Client::default();
    Ok(client)
}
