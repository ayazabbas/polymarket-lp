# claude.md — AI Agent Context

## Project
Polymarket LP bot in Rust. Farms liquidity rewards by posting two-sided limit orders around the midpoint.

## Current Phase
**Phase 0: Project Scaffold & Read-Only Client** — Not started

## Status
- [x] Research complete, plan.md written
- [ ] Cargo project initialized
- [ ] Config module
- [ ] Auth flow
- [ ] Market scanner
- [ ] Market ranking
- [ ] CLI

## Key Technical Decisions
- **Language:** Rust (user preference)
- **SDK:** `polymarket-client-sdk` v0.3 — official Polymarket Rust client
  - Features needed: `clob`, `ws`, `gamma`, `data`, `heartbeats`, `ctf`, `tracing`
  - Type-level state machine for auth (compile-time safety)
  - Re-exports alloy primitives, rust_decimal, chrono
- **Config:** TOML via `serde` + `toml`
- **CLI:** `clap` for subcommands (`scan`, `run`, `status`)
- **Async:** `tokio`
- **Auth model:** L1 (EIP-712 wallet sig) → L2 (HMAC API keys) — SDK handles this

## Architecture Notes
- One `QuoteEngine` per market, managed by a `MarketManager`
- WebSocket for real-time book data (Phase 3), REST polling initially (Phase 1-2)
- Midpoint = (best bid + best ask) / 2, size-cutoff-adjusted by Polymarket
- Use `/midpoint` API endpoint rather than computing our own
- Scoring: quadratic `S(v,s) = ((v-s)/v)²` — tighter = exponentially better
- Two-sided mandatory outside 0.10-0.90 range
- Tick sizes vary per market — must query and align

## Gotchas & Lessons
- Tick size non-compliance → order rejected
- Orders below `min_incentive_size` don't score
- Outside `max_incentive_spread` → score 0
- `heartbeats` feature: if client disconnects, ALL open orders auto-cancelled (safety net)
- Rate limits: 3,500 orders/10s burst, 36,000/10min sustained
- Fee-enabled markets (crypto 5/15min, NCAAB, Serie A): makers pay 0 fees, earn rebates
- Relayer `/submit` only 25 req/min (for CTF split/merge operations)

## File Structure (planned)
```
polymarket-lp/
├── Cargo.toml
├── plan.md
├── claude.md
├── config.example.toml
├── src/
│   ├── main.rs          # CLI entry point
│   ├── config.rs        # TOML config parsing
│   ├── client.rs        # Polymarket client wrapper (auth + init)
│   ├── scanner.rs       # Market discovery + ranking
│   ├── quoter.rs        # Quote generation + offset calculation
│   ├── engine.rs        # Per-market quoting engine
│   ├── manager.rs       # Multi-market orchestration
│   ├── risk.rs          # Inventory + risk management
│   ├── inventory.rs     # CTF split/merge/redeem operations
│   └── metrics.rs       # Logging, PnL tracking
```

## Last Updated
Phase 0 — plan.md written, project not yet initialized.
