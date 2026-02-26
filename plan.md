# Polymarket LP Bot — Plan

A Rust bot that farms Polymarket's liquidity rewards by posting two-sided limit orders around the midpoint, earning spread capture, liquidity rewards, and maker rebates.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                      polymarket-lp                       │
├──────────┬──────────┬──────────┬──────────┬─────────────┤
│  config  │  market  │  quoter  │  risk    │  dashboard  │
│          │ scanner  │          │ manager  │  / metrics  │
└──────────┴──────────┴──────────┴──────────┴─────────────┘
         │              │              │
         ▼              ▼              ▼
   ┌──────────┐  ┌───────────┐  ┌──────────┐
   │ Gamma API│  │ CLOB API  │  │ WebSocket│
   │ (markets)│  │ (orders)  │  │ (books)  │
   └──────────┘  └───────────┘  └──────────┘
```

## Key Concepts (from Polymarket docs)

### Liquidity Rewards
- Scored every minute (10,080 samples/epoch = 1 week)
- Quadratic scoring: `S(v,s) = ((v-s)/v)² × b` — tighter to midpoint = exponentially more points
- Two-sided bonus: `Q_min = min(Q_bid, Q_ask)` — single-sided divided by 3 (and 0 outside 0.10-0.90)
- Score scales linearly with order size
- `min_incentive_size` and `max_incentive_spread` per market (via CLOB/Gamma API)
- Rewards paid daily at midnight UTC in USDC
- Minimum payout: $1

### Maker Rebates (fee-enabled markets only)
- 15-min crypto, 5-min crypto, NCAAB, Serie A
- Makers pay 0 fees; takers pay `fee = C × feeRate × (p×(1-p))^exponent`
- Rebates: 20% (crypto) / 25% (sports) of collected taker fees, distributed daily
- Fee-curve weighted: your share = your_fee_equivalent / total_fee_equivalent

### Revenue Layers
1. **Spread capture** — buy low (bid filled), sell high (ask filled)
2. **Liquidity rewards** — daily USDC for resting orders near midpoint
3. **Maker rebates** — share of taker fees on fee-enabled markets

### API & SDK
- Official Rust SDK: `polymarket-client-sdk` v0.3 (crates.io)
- Features: `clob`, `ws`, `gamma`, `data`, `heartbeats`, `ctf`, `tracing`
- Type-level state machine (compile-time auth enforcement)
- `alloy` signer support (LocalSigner, KMS)
- WebSocket: `ws` feature for real-time orderbook + price + user events
- Gamma API: market/event discovery, metadata, tags
- Auth: L1 (EIP-712 wallet sig) → L2 (HMAC API keys)
- Rate limits: 3,500 orders/10s burst, 36,000/10min sustained; 1,500 midpoint/10s

### Inventory Management
- Split USDC.e → YES + NO token pairs (CTF contract on Polygon)
- Merge pairs back to USDC.e (reduce exposure / free capital)
- Redeem winning tokens after resolution ($1 each)
- Gasless via Relayer Client
- `ctf` feature in SDK handles split/merge/redeem

### Tick Sizes
- Per-market: `"0.1"`, `"0.01"`, `"0.001"`, or `"0.0001"`
- Must conform or order is rejected

---

## Phase 0: Project Scaffold & Read-Only Client
**Goal:** Cargo project, config, authenticated client, fetch market data

### Tasks
- [ ] 0.1 — Init Cargo workspace with `polymarket-client-sdk` deps (features: `clob`, `ws`, `gamma`, `data`, `heartbeats`, `ctf`, `tracing`)
- [ ] 0.2 — Config module: TOML config file for private key path/env var, RPC settings, strategy params (offset, size, requote interval, inventory caps)
- [ ] 0.3 — Auth flow: read private key → LocalSigner → authenticate CLOB client
- [ ] 0.4 — Market scanner: fetch all active rewarded markets via Gamma API, extract `min_incentive_size`, `max_incentive_spread`, `reward_amount`, fee_rate, tick_size
- [ ] 0.5 — Market ranking: score markets by `reward_amount / existing_liquidity` ratio to find best opportunities
- [ ] 0.6 — CLI: `polymarket-lp scan` command to display ranked markets in a table
- [ ] 0.7 — Update claude.md

---

## Phase 1: Midpoint Quoting Engine (Single Market, Dry Run)
**Goal:** Core quoting logic with dry-run mode (log orders without submitting)

### Tasks
- [ ] 1.1 — Midpoint tracker: poll `/midpoint` endpoint, detect shifts > threshold
- [ ] 1.2 — Offset calculator: base offset from config, adjust for tick size, fee-aware widening for fee-enabled markets (`offset = max(min_offset, taker_fee_at_midpoint / 2 + base_spread)`)
- [ ] 1.3 — Quote generator: compute bid/ask prices for YES and NO tokens (both sides of the market), validate against tick size
- [ ] 1.4 — Scoring estimator: calculate expected `S(v,s)` score for proposed quotes to help tune offsets
- [ ] 1.5 — Dry-run mode: log computed orders to stdout/file without placing them
- [ ] 1.6 — Requoting logic: cancel-and-replace on midpoint shift > offset/2 or on timer (configurable 10-60s)
- [ ] 1.7 — Unit tests for offset calculation, score estimation, tick size alignment
- [ ] 1.8 — Update claude.md

---

## Phase 2: Live Order Placement (Single Market)
**Goal:** Actually place and manage orders on one market

### Tasks
- [ ] 2.1 — Order placement: `create_and_post_order` GTC orders via SDK, batch with `post_orders` (up to 15)
- [ ] 2.2 — Order tracking: maintain local order state (open, filled, cancelled), reconcile with API
- [ ] 2.3 — Cancel-before-requote: cancel stale orders before placing new ones (batch cancel)
- [ ] 2.4 — Heartbeat: enable `heartbeats` feature so disconnect → auto-cancel all orders
- [ ] 2.5 — Fill detection: poll or subscribe (WS) for fill events, update local inventory
- [ ] 2.6 — Position tracking: track net inventory per token (YES/NO balance)
- [ ] 2.7 — Graceful shutdown: cancel all orders on SIGINT/SIGTERM
- [ ] 2.8 — Integration test: run against a low-value fee-free market with minimal size
- [ ] 2.9 — Update claude.md

---

## Phase 3: WebSocket & Real-Time Requoting
**Goal:** Replace polling with WebSocket for sub-second midpoint updates

### Tasks
- [ ] 3.1 — WebSocket connection: subscribe to orderbook channel for target market using `ws` feature
- [ ] 3.2 — Real-time midpoint: compute midpoint from live book updates, trigger requote on threshold
- [ ] 3.3 — User channel subscription: real-time fill/cancel notifications
- [ ] 3.4 — Reconnection logic: auto-reconnect on disconnect with exponential backoff
- [ ] 3.5 — Fallback: if WS disconnects, fall back to REST polling until reconnected
- [ ] 3.6 — Update claude.md

---

## Phase 4: Inventory & Risk Management
**Goal:** Prevent dangerous inventory accumulation, manage capital

### Tasks
- [ ] 4.1 — Inventory caps: configurable max net position per token (e.g., ±5,000 shares)
- [ ] 4.2 — Quote skewing: when inventory is imbalanced, tighten the reducing side and widen the accumulating side
- [ ] 4.3 — Inventory-based pause: stop quoting a side if at cap
- [ ] 4.4 — Split/merge operations: split USDC.e when token inventory low, merge pairs to free capital (via `ctf` feature)
- [ ] 4.5 — Resolution handling: detect market resolution, cancel orders, redeem winning tokens
- [ ] 4.6 — Holding rewards awareness: factor in ~4% APY on held tokens near resolution
- [ ] 4.7 — Capital allocation: distribute USDC across markets based on reward-weighted scoring
- [ ] 4.8 — Update claude.md

---

## Phase 5: Multi-Market Execution
**Goal:** Run the bot across many markets simultaneously

### Tasks
- [ ] 5.1 — Market manager: manage multiple QuoteEngine instances, one per market
- [ ] 5.2 — Periodic rescan: discover new rewarded/sponsored markets hourly, add/remove markets dynamically
- [ ] 5.3 — Capital allocation: split total capital across markets weighted by reward/liquidity ratio
- [ ] 5.4 — Aggregate risk: total portfolio inventory limits, cross-market exposure tracking
- [ ] 5.5 — Rate limit awareness: respect 3,500 orders/10s burst, 36,000/10min; queue/batch orders across markets
- [ ] 5.6 — Sponsored market detection: alert on new sponsor rewards (high reward/competition ratio opportunities)
- [ ] 5.7 — Update claude.md

---

## Phase 6: Monitoring, Metrics & Dashboard
**Goal:** Observability and performance tracking

### Tasks
- [ ] 6.1 — Structured logging: `tracing` with JSON output, log all orders/fills/cancels/errors
- [ ] 6.2 — Metrics: track PnL (spread + rewards + rebates), fill rate, uptime %, inventory per market
- [ ] 6.3 — Reward tracking: fetch daily reward payouts, compare expected vs actual
- [ ] 6.4 — Alerting: Telegram notifications for errors, large fills, reward drops, resolution events
- [ ] 6.5 — CLI dashboard: `polymarket-lp status` showing live markets, positions, PnL, open orders
- [ ] 6.6 — Persistence: SQLite or JSON for historical PnL, fills, reward payouts
- [ ] 6.7 — Update claude.md

---

## Phase 7: Advanced Strategies (Stretch)
**Goal:** Optimize returns beyond basic midpoint quoting

### Tasks
- [ ] 7.1 — Dynamic offset: widen during volatile periods (midpoint jump >5¢/min), tighten during calm
- [ ] 7.2 — Time-of-day patterns: adjust quoting around known volume spikes (e.g., crypto market hours)
- [ ] 7.3 — GTD orders: auto-expire quotes before known events/resolution using GTD order type
- [ ] 7.4 — Extreme-price farming: specialized logic for markets <10¢ or >90¢ (must be two-sided, less competition)
- [ ] 7.5 — Cross-market hedging: if quoting correlated markets, hedge inventory across them
- [ ] 7.6 — Backtester: replay historical book data to test strategy parameters
- [ ] 7.7 — Update claude.md

---

## Config Schema (draft)

```toml
[wallet]
private_key_env = "POLYMARKET_PRIVATE_KEY"
signature_type = "eoa"  # eoa | proxy | gnosis_safe

[strategy]
base_offset_cents = 1.0        # Base spread from midpoint in cents
min_offset_cents = 0.5         # Minimum offset
requote_interval_secs = 30     # Timer-based requote
requote_threshold_cents = 0.5  # Midpoint shift trigger
order_size = 500               # Shares per side per level
num_levels = 2                 # Number of price levels per side
inventory_cap = 5000           # Max net position per token

[markets]
mode = "auto"                  # auto (scan + rank) | manual (explicit list)
max_markets = 20               # Max concurrent markets
min_reward_daily = 5.0         # Minimum daily reward to bother with ($)
prefer_fee_enabled = true      # Prioritize fee-enabled for rebates

[risk]
max_total_capital = 2000.0     # Total USDC to deploy
max_per_market = 500.0         # Max capital per market
kill_switch_loss = 100.0       # Cancel everything if loss exceeds this

[monitoring]
log_level = "info"
telegram_bot_token = ""
telegram_chat_id = ""
```

---

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `polymarket-client-sdk` | Official Polymarket Rust SDK (CLOB, WS, Gamma, Data, CTF) |
| `tokio` | Async runtime |
| `alloy` | Ethereum primitives, signers (re-exported by SDK) |
| `rust_decimal` | Precise decimal arithmetic (re-exported by SDK) |
| `serde` / `toml` | Config parsing |
| `tracing` / `tracing-subscriber` | Structured logging |
| `clap` | CLI argument parsing |
| `anyhow` | Error handling |

---

## References

- [Polymarket Liquidity Rewards Docs](https://docs.polymarket.com/market-makers/liquidity-rewards)
- [Polymarket Trading / MM Best Practices](https://docs.polymarket.com/market-makers/trading)
- [Polymarket Fees](https://docs.polymarket.com/trading/fees)
- [Polymarket Maker Rebates](https://docs.polymarket.com/market-makers/maker-rebates)
- [Polymarket Inventory Management](https://docs.polymarket.com/market-makers/inventory)
- [Polymarket Rate Limits](https://docs.polymarket.com/api-reference/rate-limits)
- [Polymarket Auth](https://docs.polymarket.com/api-reference/authentication)
- [Polymarket Fetching Markets (Gamma API)](https://docs.polymarket.com/market-data/fetching-markets)
- [Rust SDK (rs-clob-client)](https://github.com/Polymarket/rs-clob-client)
- [Rust SDK on crates.io](https://crates.io/crates/polymarket-client-sdk)
- [Permissionless Liquidity Sponsorship (Feb 2026)](https://defirate.com/news/polymarket-launches-public-api-unlocks-permissionless-liquidity/)
- [Midpoint Bot Strategy (DEV.to)](https://dev.to/benjamin_martin_749c1d57f/building-a-midpoint-trading-bot-strategy-for-polymarket-fee-considered-market-making-in-2026-4lbc)
