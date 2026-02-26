# claude.md — AI Agent Context

## Project
Polymarket LP bot in Rust. Farms liquidity rewards by posting two-sided limit orders around the midpoint on Polymarket prediction markets. Three revenue layers: spread capture, daily liquidity rewards (USDC), and maker rebates on fee-enabled markets.

## Current State
**All phases 0-6 complete.** Phase 7 (Advanced Strategies — dynamic offsets, GTD orders, extreme-price farming, backtester) is stretch goals, not yet implemented.

### Phase Completion
- [x] Phase 0: Project Scaffold & Read-Only Client
- [x] Phase 1: Midpoint Quoting Engine (Dry Run)
- [x] Phase 2: Live Order Placement (Single Market)
- [x] Phase 3: WebSocket & Real-Time Requoting
- [x] Phase 4: Inventory & Risk Management
- [x] Phase 5: Multi-Market Execution
- [x] Phase 6: Monitoring, Metrics & Dashboard
- [ ] Phase 7: Advanced Strategies (Stretch — not started)

### Build & Test
- **Compiles:** Yes (`cargo build` succeeds)
- **Tests:** 24 unit tests passing (`cargo test`)
- **Not yet tested against live Polymarket** — needs wallet with USDC.e on Polygon

---

## How to Run

```bash
# 1. Copy and edit config
cp config.example.toml config.toml
# Set POLYMARKET_PRIVATE_KEY env var (or change private_key_env in config)

# 2. Scan markets (no wallet needed — read-only)
cargo run -- scan -n 20

# 3. Dry-run on a single market (logs quotes without placing orders)
cargo run -- run --market <condition_id>

# 4. Live single-market
cargo run -- run --live --market <condition_id>

# 5. Multi-market auto mode
cargo run -- run --live --multi

# 6. Check status / dashboard
cargo run -- status
```

### Environment Variables
- `POLYMARKET_PRIVATE_KEY` — Polygon wallet private key (hex, no 0x prefix)
- `RUST_LOG` — tracing filter (default from config: `info`)

---

## Architecture

### Data Flow
```
Gamma API → scanner.rs (discover + rank markets)
                ↓
         manager.rs (allocate capital, spawn engines)
                ↓
    ┌─── engine.rs (one per market) ───┐
    │                                    │
    │  ws.rs ←→ midpoint updates         │
    │  quoter.rs → compute bid/ask       │
    │  risk.rs → skew/cap check          │
    │  orders.rs → place/cancel via CLOB │
    │  metrics.rs → track PnL/fills      │
    └────────────────────────────────────┘
```

### Core Loop (per market, in engine.rs)
1. Get midpoint (WebSocket if connected, REST fallback)
2. Check if requote needed (midpoint shift > threshold OR timer expired)
3. Consult risk.rs for inventory skew and caps
4. Generate quotes via quoter.rs (fee-aware offset, tick-aligned, multi-level)
5. Cancel stale orders, place new ones via orders.rs
6. Track fills, update inventory and PnL in metrics.rs

### Multi-Market (manager.rs)
- Spawns one `QuoteEngine` per selected market
- Capital allocation weighted by `reward / liquidity` ratio
- Rate limiter: respects 3,500 orders/10s burst, 36,000/10min sustained
- Hourly rescan via Gamma API to discover new/sponsored markets
- Sponsored market detection (high reward/competition ratio = alpha)

---

## File Structure & Responsibilities

| File | LOC | Purpose |
|------|-----|---------|
| `main.rs` | 489 | CLI (`clap`): `scan`, `run`, `status` subcommands. Run loops, signal handling, table output. |
| `config.rs` | 229 | TOML config parsing with serde. Sections: wallet, strategy, markets, risk, monitoring. All fields have sensible defaults. |
| `client.rs` | 54 | Creates authenticated CLOB client + Gamma client. Handles `LocalSigner` → `authentication_builder` → `authenticate()` flow. |
| `scanner.rs` | 181 | Fetches active markets from Gamma API. Filters by rewards availability. Ranks by `reward_amount / liquidity` ratio. Returns `Vec<MarketInfo>`. |
| `quoter.rs` | 260 | Core quoting math. `compute_offset()` does fee-aware offset calculation. `generate_quotes()` produces multi-level bid/ask with tick alignment. `estimate_score()` predicts reward scoring. Handles inventory skew adjustments. |
| `engine.rs` | 384 | Per-market state machine. Holds `MarketInfo`, current quotes, tracked orders, inventory, PnL. `tick()` is the main loop step. Handles WS events (midpoint update, fill, cancel). |
| `orders.rs` | 246 | Order lifecycle: create `SignableOrder` → sign → post (single or batch up to 15). Cancel by ID or cancel-all. `TrackedOrder` struct for local state reconciliation. |
| `ws.rs` | 220 | WebSocket manager using SDK's `ws` feature. Subscribes to midpoint + book channels. Auto-reconnect with exponential backoff (1s → 2s → 4s → ... → 30s cap). Falls back to REST on disconnect. Emits `WsEvent` enum. |
| `risk.rs` | 305 | Inventory caps (configurable per-token max). Quote skewing formula: widens accumulating side, tightens reducing side proportional to inventory/cap ratio. Kill switch: cancels everything if loss exceeds threshold. Capital allocation across markets. |
| `inventory.rs` | 102 | CTF operations interface: split USDC.e → YES+NO, merge pairs → USDC.e, redeem after resolution. Uses SDK's `ctf` feature. Balance checking. |
| `manager.rs` | 379 | Multi-market orchestrator. Token-bucket rate limiter. Periodic rescan (hourly). Manages engine lifecycle (add/remove markets). Aggregate portfolio risk. |
| `metrics.rs` | 325 | PnL tracking (spread P&L + estimated rewards + rebates). Fill rate calculation. JSON persistence to `metrics.json`. Telegram alerting via bot API. Dashboard output for `status` command. |

---

## Key Technical Decisions

| Decision | Choice | Why |
|----------|--------|-----|
| Language | Rust | User preference; SDK available |
| SDK | `polymarket-client-sdk` v0.4.3 | Official Polymarket crate, type-safe auth state machine, re-exports alloy/decimal/chrono |
| Async | tokio | SDK is async-first with reqwest |
| Config | TOML + serde | Simple, readable, good Rust ecosystem support |
| CLI | clap (derive) | Standard Rust CLI framework |
| Decimals | `rust_decimal` | Precise arithmetic for prices (re-exported by SDK) |
| Persistence | JSON files | Simple, no DB dependency for v1 |
| Alerting | Telegram bot API | Ayaz uses Telegram |

### SDK Features Used
```toml
polymarket-client-sdk = { version = "0.4", features = [
    "clob",        # Core CLOB client (orders, market data, auth)
    "ws",          # WebSocket streaming (midpoint, book, user events)
    "gamma",       # Market/event discovery via Gamma API
    "data",        # Positions, trades, analytics
    "heartbeats",  # Auto-cancel orders on disconnect (safety)
    "ctf",         # Split/merge/redeem outcome tokens
    "tracing",     # Structured logging for SDK internals
]}
```

---

## SDK API Quick Reference

```rust
// Auth flow
let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
let client = Client::new("https://clob.polymarket.com", Config::default())?
    .authentication_builder(&signer)
    .authenticate()
    .await?;

// Market data (no auth needed)
client.midpoint(&MidpointRequest { token_id }) → MidpointResponse { mid: Decimal }
client.book(&BookRequest { token_id }) → BookResponse { bids, asks, ... }
client.tick_size(&TickSizeRequest { token_id }) → TickSizeResponse
client.fee_rate(&FeeRateRequest { token_id }) → FeeRateResponse

// Order flow (needs auth)
let order = client.limit_order().token_id(id).side(Side::Buy).price(p).size(s).build().await?;
let signed = client.sign(&signer, order).await?;
client.post_order(signed) → PostOrderResponse
client.post_orders(vec![...]) → batch (up to 15)
client.cancel_order(order_id)
client.cancel_all_orders()

// Gamma API (no auth)
let gamma = gamma::Client::default();
gamma.markets(&MarketsRequest { active: true, closed: false, ... }) → Vec<Market>

// WebSocket
let ws = clob::ws::Client::default();
ws.subscribe_midpoints(asset_ids) → Stream<MidpointUpdate>

// Gamma Market fields for rewards:
// condition_id: Option<B256>, clob_token_ids: Option<Vec<U256>>
// rewards_min_size, rewards_max_spread, competitive
```

---

## Quoting Math

### Offset Calculation (`quoter.rs`)
```
base_offset = config.base_offset_cents / 100  (convert cents → price)

// Fee-aware adjustment for fee-enabled markets:
fee_at_midpoint = fee_rate × p × (1-p)  // peaks at p=0.50
fee_offset = fee_at_midpoint / 2 + base_offset

final_offset = max(min_offset, fee_offset)
```

### Inventory Skew (`risk.rs`)
```
skew_ratio = net_inventory / inventory_cap  // -1.0 to 1.0
bid_offset = offset × (1 + skew_ratio × skew_factor)   // wider if long
ask_offset = offset × (1 - skew_ratio × skew_factor)   // tighter if long
```

### Reward Score Estimation (`quoter.rs`)
```
S(v, s) = ((v - s) / v)²
where v = max_incentive_spread, s = distance from midpoint
```
Quadratic — being 1¢ away scores 4× more than 2¢ away.

---

## Gotchas & Lessons Learned

- **Tick size compliance is mandatory** — orders rejected if price doesn't align. Always round to tick.
- **`min_incentive_size` check** — orders below this don't score for rewards. Config `order_size` must exceed it.
- **Outside `max_incentive_spread`** → reward score = 0. Offset must stay within.
- **Two-sided mandatory outside 0.10-0.90** — single-sided scores 0 at extreme prices.
- **Heartbeats feature** — if client disconnects, ALL open orders auto-cancelled. Good safety net but means uptime matters.
- **Rate limits** — 3,500 POST orders/10s burst, 36,000/10min sustained. Relayer `/submit` only 25/min (for CTF ops).
- **Gamma `condition_id` is `Option<B256>`** and `clob_token_ids` is `Option<Vec<U256>>` — need to handle None cases.
- **Fee-enabled markets** (crypto 5/15min, NCAAB, Serie A): makers pay 0 fees + earn 20-25% of taker fees as rebates. Prioritize these.
- **Permissionless reward sponsorship** (new Feb 2026): anyone can sponsor rewards on any market. Sponsored niche markets with few makers = outsized rewards.
- **Midpoint** = `(best_bid + best_ask) / 2`, size-cutoff-adjusted. Use the API endpoint, don't compute your own.

---

## What's NOT Implemented (Phase 7 / Future)

- Dynamic offset widening during volatile periods
- GTD (Good-Till-Date) orders for event-aware expiry
- Extreme-price farming (<10¢ / >90¢ specialized logic)
- Cross-market inventory hedging
- Historical backtester
- Time-of-day volume pattern awareness

---

## Config Reference

See `config.example.toml` for full schema. Key sections:

| Section | Key Fields |
|---------|-----------|
| `[wallet]` | `private_key_env`, `signature_type` (eoa/proxy/gnosis_safe) |
| `[strategy]` | `base_offset_cents` (1.0), `order_size` (500), `num_levels` (2), `requote_interval_secs` (30), `inventory_cap` (5000) |
| `[markets]` | `mode` (auto/manual), `max_markets` (20), `min_reward_daily` (5.0), `prefer_fee_enabled` (true) |
| `[risk]` | `max_total_capital` (2000), `max_per_market` (500), `kill_switch_loss` (100) |
| `[monitoring]` | `log_level`, `telegram_bot_token`, `telegram_chat_id` |

---

## References

- [plan.md](./plan.md) — Full project plan with all phase details
- [Polymarket Liquidity Rewards](https://docs.polymarket.com/market-makers/liquidity-rewards) — Scoring formula
- [Polymarket Trading](https://docs.polymarket.com/market-makers/trading) — Order types, best practices
- [Polymarket Fees](https://docs.polymarket.com/trading/fees) — Fee curves, maker rebates
- [Rust SDK](https://github.com/Polymarket/rs-clob-client) — Source, examples, feature flags
- [Rust SDK crate](https://crates.io/crates/polymarket-client-sdk) — v0.4.3

## Last Updated
2026-02-26 — All phases 0-6 complete. 24 tests passing. Not yet tested against live Polymarket.
