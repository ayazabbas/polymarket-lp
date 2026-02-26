# claude.md — AI Agent Context

## Project
Polymarket LP bot in Rust. Farms liquidity rewards by posting two-sided limit orders around the midpoint.

## Current Phase
**All phases 0-6 complete.** Phase 7 (Advanced Strategies) is stretch goals, skipped.

## Status
- [x] Phase 0: Project Scaffold & Read-Only Client
- [x] Phase 1: Midpoint Quoting Engine (Dry Run)
- [x] Phase 2: Live Order Placement (Single Market)
- [x] Phase 3: WebSocket & Real-Time Requoting
- [x] Phase 4: Inventory & Risk Management
- [x] Phase 5: Multi-Market Execution
- [x] Phase 6: Monitoring, Metrics & Dashboard
- [ ] Phase 7: Advanced Strategies (Stretch — skipped)

## Key Technical Decisions
- **SDK:** `polymarket-client-sdk` v0.4.3 — type-level auth state machine
- **Config:** TOML via `serde` + `toml`
- **CLI:** `clap` — subcommands: `scan`, `run`, `status`
- **Persistence:** JSON via `serde_json` for metrics
- **Alerting:** Telegram bot API via `reqwest`

## SDK API Quick Reference
- `clob::Client::new(host, config)` → unauthenticated
- `.authentication_builder(&signer).authenticate().await` → authenticated
- `.midpoint(&MidpointRequest)` → `MidpointResponse { mid: Decimal }`
- `.limit_order().token_id().side().price().size().build().await` → `SignableOrder`
- `.sign(&signer, order).await` → `SignedOrder`
- `.post_order(signed)` / `.post_orders(vec)` → `PostOrderResponse`
- `.cancel_order(id)` / `.cancel_all_orders()`
- `gamma::Client::default()` then `.markets(&MarketsRequest)`
- WS: `clob::ws::Client::default()`, `.subscribe_midpoints(asset_ids)`
- Gamma Market: `condition_id` is `Option<B256>`, `clob_token_ids` is `Option<Vec<U256>>`
- Gamma Market reward fields: `rewards_min_size`, `rewards_max_spread`, `competitive`

## File Structure
```
src/
├── main.rs          # CLI entry, run loops (scan/run/run --multi/status)
├── config.rs        # TOML config: wallet, strategy, markets, risk, monitoring
├── client.rs        # CLOB + Gamma client creation, auth flow
├── scanner.rs       # Gamma API market fetch, filtering, ranking
├── quoter.rs        # Offset calc, tick alignment, quote generation, score estimation
├── engine.rs        # Per-market quoting engine (dry-run + live + WS event handling)
├── orders.rs        # Order placement, cancellation, tracking, reconciliation
├── ws.rs            # WebSocket manager (midpoint, book, user events, reconnect)
├── risk.rs          # Inventory caps, skewing, kill switch, capital allocation
├── inventory.rs     # CTF split/merge/redeem interfaces, balance checking
├── manager.rs       # Multi-market orchestration, rate limiter, rescan
└── metrics.rs       # PnL tracking, fill rates, persistence, Telegram alerts, dashboard
```

## Test Coverage
24 unit tests: config(2), scanner(2), quoter(8), risk(5), manager(2), metrics(4), + 1 persistence

## Last Updated
All phases 0-6 complete. 24 tests passing. Build succeeds.
