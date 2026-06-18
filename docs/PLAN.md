# shadowcat — Milestone Roadmap

MVP-first. Phase 1 ends at a playable dogfood alpha. Later phases add table features, atmosphere, then platform/scale. Each milestone lists its goal, key deliverables, and explicit exclusions. Architecture and rationale live in [`design/ARCHITECTURE.md`](design/ARCHITECTURE.md).

Guiding rule: build what you cannot build on top of. Networking and permissions precede features; features precede polish; the module API stays 0.x until external evidence proves it.

## Phase 1 — MVP (→ dogfood alpha)

### M1 · Project infrastructure
- Monorepo (server / client / modules / types); Cargo + pnpm workspaces; Vite.
- CI: Rust tests, TS typecheck, lint, cargo-bloat budget.
- ts-rs type pipeline (Rust→TS), CI-enforced sync.
- SQLite-only data target. Release `opt-level="z"`.
- Excludes: Postgres, Tantivy, zstd, blake3.

### M2 · Data foundation
- Document envelope + opaque `system` body + `schema_version`; in-memory `migrateData` coercion.
- Permission schema (document-level roles; property-level overrides).
- Per-world atomic sequence counter.
- Undoable mutation representation (command/event records) — invariant 8.
- Database layer + unit tests; no HTTP yet.

### M3 · Auth + server skeleton
- axum boots, runs migrations; argon2 + tower-sessions; server / GM / player roles.
- Structured logging (tracing), request ids, `/health`.
- Single-binary build (client bundle embedded via `rust-embed`; stub bundle acceptable here).

### M4 · WebSocket event bus
- Per-world rooms; sequenced broadcasts; time-bounded event ring buffer; client sequence guard; reconnect/resync.
- Server time source + client offset calibration (ahead of need).
- Observability + desync telemetry; spawnable test-server binary; desync-convergence test harness — the project's highest-value test.

### M5 · Document CRUD + permissions + migration + rollback
- CRUD over HTTP + WS; `PermissionContext` (per-recipient filtering, property-level stripping).
- Field-path updates + field-level merge; intent/confirm + optimistic rollback UX.
- Client document store; in-memory schema coercion + migration status.
- Compendium / world / embedded copy independence.

### M6 · Headless core client
- WS client (reconnect / backoff / sequence guard); Zod-validated document store.
- Versioned hook system (informational / mutating / cancellable); service registry + middleware.
- Module manifest + loader (topo-sort, semver, hot-unload cleanup); local module registry.
- Framework-neutral mount points; `Core.search` API (over FTS5). Module API explicitly 0.x.
- No UI. Integration tests via the M4 test-server.

### M7 · Layout-lite + theming scaffold
- Fixed panel layout; one dark theme via the 3-tier SCSS token system; i18n scaffold.
- Session state persisted in the DB.
- Excludes: drag-resize, pop-out / multi-window, multi-theme, user themes, module styling modes.

### M8 · ECS + scene rendering
- hecs hydration/mutation boundary; ECS→WS dispatch.
- Render-layer / filter abstraction spiked against the vision mask path first.
- Scene load, grid (square / hex), camera pan/zoom; basic token placement; measurement / template / drawing tools; pings.
- Minimal raw asset upload + static serving (scene backgrounds, token art); no conversion — the full asset pipeline is Phase 2.
- Excludes: post-processing, multi-level maps, portals.

### M9 · Walls + vision + fog
- Vector walls as ECS components; movement blocking.
- Rust raycasting; per-player visibility polygons (`geo` union); PixiJS masks; persistent fog of war.
- GM vision mode. Server-authoritative geometric vision only.
- Excludes: photometric / illumination coupling, darkvision / tremorsense / height, Web-Worker optimistic vision.

### M10 · Tokens
- Actor-linked tokens; shapes; instanced / unique modes; A* pathfinding with waypoints; status conditions; factions.

### M11 · Dice + chat
- From-scratch dice engine (notation, modifiers, advantage/disadvantage, DCs, success counting, tiers); hook integration; sequenced results.
- Chat log; whispers (user-to-user / GM-only).

### M12 · Minimal default modules
- Actor / scene browsers, generic actor / item sheets, chat panel — built against the public API, each treated as an API bug report.

**▶ Dogfood alpha gate** — backups must exist before real worlds accrue.

## Phase 2 — Full table
Combat tracker (initiative, hidden combatants, turn-event triggers; depends on M11 dice) → real asset pipeline (chunked upload, tags, derived tags, conversion options) → layout / theming completion (drag-resize, pop-out, multi / user themes, module styling modes) → vision / lighting completion (photometric, darkvision / tremorsense / height) → token enrichment (aura / light / sound / VFX emitters, trigger regions, token-art) → full default module suite → search consolidated into one milestone (single backend; no three-backend split).

## Phase 3 — Atmosphere
Audio (mixer, channels, playlists, world-clock sync; then spatial + wall occlusion; transcode via `symphonia` + `opus`/`vorbis_rs`) → VFX (sprite effects, concurrent SFX) → multi-level maps + portals.

## Phase 4 — Platform & scale
Trusted local modding hardening → freeze the module API on evidence (≥1 external module ships without core patches) → [only if a marketplace is pursued] WASM sandbox + registry + signing / SRI / CSP + package browser → native wrappers (Tauri 2, Capacitor) → hardening + distribution (backup scheduling, world snapshots, WS load + resync stress tests, rate limiting, rustls-acme TLS, Steam OpenID + plain-executable distribution).

## Cross-cutting (not deferred)
- Observability + desync telemetry: M4.
- Desync-convergence test: M4, maintained throughout.
- Backups: before the dogfood alpha gate (end of Phase 1), not Phase 4.
- Rate limiting on WS / upload: introduced with the surfaces it protects, not only at hardening.
- Error UX (disconnect, rejected optimistic op, failed upload): owned by M5 / M6 client work.
- Account model: self-host admin-creates-users (confirm).
