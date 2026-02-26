# claude.md — AI Agent Context

## Project
Polymarket LP bot in Rust. Farms liquidity rewards by posting two-sided limit orders around the midpoint.

## Current Phase
**Phase 1: Midpoint Quoting Engine (Dry Run)** — In progress

## Status
- [x] Phase 0: Project Scaffold & Read-Only Client
  - [x] Cargo project initialized with polymarket-client-sdk v0.4
  - [x] Config module (TOML parsing, all strategy/risk/market params)
  - [x] Auth flow (LocalSigner → CLOB authenticate)
  - [x] Market scanner (Gamma API fetch + filtering)
  - [x] Market ranking (score by reward/liquidity ratio)
  - [x] CLI (`scan`, `run`, `status` subcommands)
- [ ] Phase 1: Midpoint Quoting Engine (Dry Run)
- [ ] Phase 2: Live Order Placement (Single Market)
- [ ] Phase 3: WebSocket & Real-Time Requoting
- [ ] Phase 4: Inventory & Risk Management
- [ ] Phase 5: Multi-Market Execution
- [ ] Phase 6: Monitoring, Metrics & Dashboard

## Key Technical Decisions
- **Language:** Rust
- **SDK:** `polymarket-client-sdk` v0.4.3 (latest) — NOT v0.3 as originally planned
  - Features: `clob`, `ws`, `gamma`, `data`, `heartbeats`, `ctf`, `tracing`
  - Type-level state machine: `Client<Unauthenticated>` → `Client<Authenticated<Normal>>`
  - Re-exports: alloy primitives (Address, B256, U256), rust_decimal, chrono
  - `bon::Builder` for all request types
  - Gamma Market `condition_id` is `Option<B256>`, `clob_token_ids` is `Option<Vec<U256>>`
- **Config:** TOML via `serde` + `toml`
- **CLI:** `clap` for subcommands (`scan`, `run`, `status`)
- **Async:** `tokio`
- **Auth model:** L1 (EIP-712) → L2 (HMAC) — SDK handles via `authentication_builder`

## Architecture Notes
- One `QuoteEngine` per market, managed by a `MarketManager`
- WebSocket for real-time book data (Phase 3), REST polling initially (Phase 1-2)
- Gamma Market fields: `rewards_min_size`, `rewards_max_spread` (NOT min_incentive_size/max_incentive_spread)
- `order_price_min_tick_size` on Gamma Market for tick size
- `competitive` field used as reward proxy for scoring

## SDK API Quick Reference
- `clob::Client::new(host, config)` → unauthenticated
- `.authentication_builder(&signer).authenticate().await` → authenticated
- `.midpoint(&MidpointRequest)` → `MidpointResponse { mid: Decimal }`
- `.limit_order().token_id().side().price().size().build().await` → `SignableOrder`
- `.sign(&signer, order).await` → `SignedOrder`
- `.post_order(signed)` / `.post_orders(vec)` → `PostOrderResponse`
- `.cancel_order(id)` / `.cancel_all_orders()`
- `gamma::Client::default()` then `.markets(&MarketsRequest)`
- WS: `clob::ws::Client::default()`, `.subscribe_orderbook(asset_ids)`

## File Structure
```
src/
├── main.rs          # CLI entry point
├── config.rs        # TOML config parsing
├── client.rs        # Polymarket client wrapper
├── scanner.rs       # Market discovery + ranking
├── quoter.rs        # Quote generation + offset calculation (Phase 1)
├── engine.rs        # Per-market quoting engine (Phase 1-2)
├── orders.rs        # Order placement + tracking (Phase 2)
├── ws.rs            # WebSocket manager (Phase 3)
├── risk.rs          # Inventory + risk management (Phase 4)
├── inventory.rs     # CTF split/merge/redeem (Phase 4)
├── manager.rs       # Multi-market orchestration (Phase 5)
└── metrics.rs       # Logging, PnL tracking (Phase 6)
```

## Last Updated
Phase 0 complete — all tests pass, cargo build succeeds.
