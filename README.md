# polymarket-lp

A Rust bot that earns passive yield on [Polymarket](https://polymarket.com) by providing liquidity. It posts two-sided limit orders around the midpoint price and collects three types of revenue:

1. **Spread capture** — buy low, sell high as orders get filled
2. **Liquidity rewards** — daily USDC payouts for keeping tight quotes on the book
3. **Maker rebates** — share of taker fees on fee-enabled markets (crypto, sports)

## How It Works

Polymarket pays market makers who post resting limit orders near the midpoint. The closer your orders, the higher your reward score ([quadratic formula](https://docs.polymarket.com/market-makers/liquidity-rewards)). This bot automates the entire process:

- Discovers rewarded markets and ranks them by opportunity (reward $ / existing liquidity)
- Posts two-sided quotes (bid + ask) on both YES and NO tokens
- Requotes when the midpoint moves or on a timer
- Manages inventory risk with skewing, caps, and a kill switch
- Tracks PnL and sends Telegram alerts

## Quick Start

### Prerequisites

- Rust 1.82+ (`rustup update`)
- A Polygon wallet with USDC.e (bridged from Ethereum if needed)
- Your wallet's private key (hex format)

### Install & Configure

```bash
git clone https://github.com/ayazabbas/polymarket-lp.git
cd polymarket-lp

# Copy config and edit
cp config.example.toml config.toml

# Set your private key
export POLYMARKET_PRIVATE_KEY="your_hex_private_key_here"
```

### Usage

```bash
# Scan markets — see what's worth providing liquidity on
cargo run -- scan

# Scan with filters
cargo run -- scan --min-reward 10 -n 50

# Dry run on a specific market (logs quotes, doesn't place orders)
cargo run -- run --market <condition_id>

# Go live on a single market
cargo run -- run --live --market <condition_id>

# Multi-market auto mode (scans, ranks, deploys capital across top markets)
cargo run -- run --live --multi

# Check current positions and PnL
cargo run -- status
```

### First Run Recommendation

1. Run `cargo run -- scan` to see available markets
2. Pick a low-stakes fee-free market
3. Run in dry-run mode first: `cargo run -- run --market <id>`
4. Watch the logs — it'll show what quotes it *would* place
5. When comfortable, add `--live` to start placing real orders

## Configuration

All settings are in `config.toml`:

### `[wallet]`
| Field | Default | Description |
|-------|---------|-------------|
| `private_key_env` | `POLYMARKET_PRIVATE_KEY` | Env var containing your private key |
| `signature_type` | `eoa` | Wallet type: `eoa`, `proxy`, or `gnosis_safe` |

### `[strategy]`
| Field | Default | Description |
|-------|---------|-------------|
| `base_offset_cents` | `1.0` | How far from midpoint to place orders (in cents) |
| `min_offset_cents` | `0.5` | Minimum offset (safety floor) |
| `requote_interval_secs` | `30` | Requote on timer even if midpoint hasn't moved |
| `requote_threshold_cents` | `0.5` | Midpoint shift that triggers immediate requote |
| `order_size` | `500` | Shares per order per level |
| `num_levels` | `2` | Price levels per side (e.g., 2 = two bids + two asks) |
| `inventory_cap` | `5000` | Max net position per token before pausing that side |

### `[markets]`
| Field | Default | Description |
|-------|---------|-------------|
| `mode` | `auto` | `auto` = scan and pick best markets; `manual` = use `manual_markets` list |
| `max_markets` | `20` | Maximum concurrent markets to quote |
| `min_reward_daily` | `5.0` | Ignore markets paying less than this per day ($) |
| `prefer_fee_enabled` | `true` | Prioritize fee-enabled markets (crypto/sports) for rebate income |

### `[risk]`
| Field | Default | Description |
|-------|---------|-------------|
| `max_total_capital` | `2000.0` | Total USDC to deploy across all markets |
| `max_per_market` | `500.0` | Maximum USDC allocated to any single market |
| `kill_switch_loss` | `100.0` | Cancel everything if total loss exceeds this |

### `[monitoring]`
| Field | Default | Description |
|-------|---------|-------------|
| `log_level` | `info` | Log verbosity: `debug`, `info`, `warn`, `error` |
| `telegram_bot_token` | *(empty)* | Telegram bot token for alerts (optional) |
| `telegram_chat_id` | *(empty)* | Telegram chat ID for alerts (optional) |

## Architecture

```
                    ┌─────────────────┐
                    │   Gamma API     │  Market discovery
                    │  (read-only)    │  + reward metadata
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │    Scanner      │  Rank by reward/liquidity
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │    Manager      │  Capital allocation
                    │                 │  Rate limiting
                    │                 │  Hourly rescan
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────▼──────┐ ┌────▼─────┐ ┌──────▼───────┐
     │  Engine (Mkt1) │ │ Engine 2 │ │  Engine N    │
     │                │ │          │ │              │
     │ • Midpoint     │ │   ...    │ │     ...      │
     │ • Quoting      │ │          │ │              │
     │ • Orders       │ │          │ │              │
     │ • Inventory    │ │          │ │              │
     │ • Risk         │ │          │ │              │
     └───────┬────────┘ └──────────┘ └──────────────┘
             │
    ┌────────▼────────┐
    │    CLOB API     │  Orders, midpoints, books
    │   + WebSocket   │  Real-time updates
    └─────────────────┘
```

Each market gets its own `QuoteEngine` that independently:
1. Tracks the midpoint (WebSocket with REST fallback)
2. Computes optimal quotes (fee-aware, tick-aligned, multi-level)
3. Applies inventory skew (widen the risky side, tighten the reducing side)
4. Places/cancels orders via the CLOB API
5. Tracks fills and PnL

## How Rewards Work

Polymarket scores your liquidity every minute using:

```
Score = ((max_spread - your_spread) / max_spread)² × order_size
```

Key points:
- **Tighter = exponentially better** — 1¢ from midpoint scores 4× more than 2¢
- **Two-sided required** — your score is `min(bid_score, ask_score)`, so quote both sides
- **Size matters** — score scales linearly with order quantity
- **Uptime matters** — sampled every minute, 10,080 samples per epoch
- Rewards paid daily at midnight UTC. Minimum payout: $1.

Fee-enabled markets (crypto 5/15min, NCAAB, Serie A) additionally pay maker rebates — 20-25% of taker fees redistributed daily to liquidity providers.

## Risk Management

- **Inventory caps** — stops quoting one side if position exceeds limit
- **Quote skewing** — automatically tightens the side that reduces inventory
- **Kill switch** — cancels all orders if total loss exceeds threshold
- **Heartbeat safety** — if the bot disconnects, Polymarket auto-cancels all open orders
- **Graceful shutdown** — Ctrl+C cancels all orders before exiting

## Monitoring

- **Structured logs** via `tracing` (JSON output available)
- **PnL tracking** — spread P&L + estimated rewards + rebates
- **Telegram alerts** — errors, large fills, kill switch triggers
- **JSON persistence** — metrics saved to `metrics.json`
- **Dashboard** — `cargo run -- status` for live overview

## Fee-Enabled Markets

Some markets charge taker fees that fund maker rebates:

| Market Type | Taker Fee (peak @ 50%) | Maker Rebate |
|-------------|----------------------|--------------|
| 15-min crypto | 1.56% | 20% of fees |
| 5-min crypto | 1.56% | 20% of fees |
| NCAAB | 0.44% | 25% of fees |
| Serie A | 0.44% | 25% of fees |

Makers always pay **zero fees**. The bot automatically widens spreads on fee-enabled markets to attract more taker flow.

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Build release
cargo build --release

# Verbose logging
RUST_LOG=debug cargo run -- scan
```

## Disclaimer

This is experimental software. Prediction markets involve risk of loss. The bot may lose money due to adverse price movements, inventory accumulation near resolution, or technical failures. Start with small amounts and monitor closely. Not financial advice.

