# shadowcat — Milestone Roadmap

MVP-first. Phase 1 ends at a playable dogfood alpha. Later phases add table features, atmosphere, then platform/scale. Each milestone lists its goal, key deliverables, and explicit exclusions. Architecture and rationale live in [`design/ARCHITECTURE.md`](design/ARCHITECTURE.md).

Guiding rule: build what you cannot build on top of. Networking and permissions precede features; features precede polish; the module API stays 0.x until evidence proves it.

## Phase 1 — MVP (→ dogfood alpha)

### M1 · Project infrastructure ✅
- Monorepo workspace under `src/` (`src/server/` Rust, `src/client/{core,ui}/`, `src/modules/`, `src/types/`); build output in `dist/`. Cargo + pnpm workspaces; Vite. Rename the empty `source/` dir to `src/`.
- CI: Rust tests, TS typecheck, lint, cargo-bloat budget.
- ts-rs type pipeline (Rust→TS), CI-enforced sync.
- SQLite-only data target. Release `opt-level="z"`.
- Excludes: Postgres, Tantivy, zstd, blake3.

### M2 · Data foundation ✅
- Document envelope + opaque `system` body + `schema_version`.
- Migration *machinery only*: the synchronous client-side `migrateData` seam exists and runs as a no-op pass-through. No actual migrations are authored in v1 (nothing ships pre-v1, so no documents exist to migrate).
- Permission schema (server / world / document roles, incl. observer / spectator; property-level overrides).
- Per-world atomic sequence counter.
- Undoable mutation representation (command/event records) — the *undoable-mutation invariant*. This reversible representation is the single shared substrate for both optimistic rollback (M5/M6) and undo; later rollback consumes it, not a parallel representation.
- Database layer + unit tests; no HTTP yet.

### M3 · Auth + server skeleton ✅
- axum boots, runs migrations; argon2 + tower-sessions; server / GM / player / spectator roles.
- Structured logging (tracing), request ids, `/health`.
- Single-binary build (client bundle embedded via `rust-embed`; stub bundle acceptable here).

### M4 · WebSocket event bus ✅
- Per-world rooms; sequenced broadcasts; time-bounded event ring buffer; client sequence guard; reconnect/resync.
- Server time source + client offset calibration (ahead of need).
- Observability + desync telemetry; spawnable test-server binary; desync-convergence test harness — the project's highest-value test.

### M5 · Document CRUD + permissions + server-side rollback ✅
- CRUD over HTTP + WS; `PermissionContext` (per-recipient filtering, property-level stripping).
- Field-path updates + field-level merge; **server-side intent/confirm + rollback-to-authoritative** (the client-side optimistic-apply + rollback UX lands in M6 with the WS client and document store).
- Compendium / world / embedded copy independence.
- Stable UUID asset references (data-model property; the upload surface itself lands in M8).

### M6 · Headless core client
Split into three sub-milestones (each brainstorm→spec→plan→execute). No UI;
integration-tested via the M4/M5 test-server. Headless core is a **Svelte-free,
framework-neutral TS module**; Module API explicitly 0.x.

#### M6a · Client core foundation ✅
- WS client (reconnect / backoff / client-side sequence guard) over the M5
  `Intent`/`Event`/`Reject` protocol.
- The **single** Zod-validated client document store (built once here).
- **Client-side optimistic-apply + rollback**, consuming the M2 reversible
  representation; resolves the M5-deferred intent correlation client-side
  (`author` + seq FIFO).

#### M6b · Modules + capabilities (declarative) ✅
> Spec: [`superpowers/specs/2026-06-18-m6b-modules-capabilities-design.md`](superpowers/specs/2026-06-18-m6b-modules-capabilities-design.md).
> Plan: [`superpowers/plans/2026-06-18-m6b-modules-capabilities.md`](superpowers/plans/2026-06-18-m6b-modules-capabilities.md).
> Server capability slice buddy-checked (two-reviewer) before merge; two Critical
> bypasses (Create-path, ancestor-write) + a `Welcome` over-share fixed.
- Versioned hook system (informational / mutating / cancellable); service
  registry + middleware.
- Module manifest + loader (topo-sort, semver, hot-unload cleanup); local
  module registry.
- **Capability Phase 2 = declarative, data-driven, field-path-scoped capability
  requirements** declared by modules/worlds + manifest capability declarations +
  client capability-awareness (`Welcome` carries world-default grants + the
  actor's role so the client replicates resolution). Server stays
  structural-only; no server-side module code.

#### M6c · Search ✅
> Decomposed into **M6c-1** (one-shot search) and **M6c-2** (live query
> subscriptions); both complete — M6c (and the **M6 milestone**) done. Spec:
> [`superpowers/specs/2026-06-18-m6c-search-design.md`](superpowers/specs/2026-06-18-m6c-search-design.md).
- **M6c-1 ✅** — FTS5 virtual table (visibility-split index: GM-only-stripped
  `content` + full `content_all`) + write-time transactional sync +
  transport-agnostic `Repository::search` (BM25 rank, per-recipient read filter,
  cursor pagination, scan cap) + WS `Search`/`SearchResult`/`SearchError`
  request/response frames on a generic correlation layer + `Core.search`.
  Search core buddy-checked (snippet/match/score confidentiality leak fixed).
  Plan: [`superpowers/plans/2026-06-18-m6c-1-search.md`](superpowers/plans/2026-06-18-m6c-1-search.md).
- **M6c-2 ✅** — live top-N search subscriptions over the M4 broadcast:
  `Search{subscribe}` registers a per-connection subscription in the egress task;
  a leading-edge 150ms-coalesced re-eval re-runs `Repository::search` (inheriting
  per-recipient filtering + the visibility-split index) and pushes `SearchUpdate`
  when an `(doc_id, score, updated_at)` fingerprint changes; 16/connection cap;
  `Unsubscribe` + disconnect cleanup; client `Core.subscribeSearch`. Egress
  engine buddy-checked (debounce-starvation fixed). Plan:
  [`superpowers/plans/2026-06-18-m6c-2-live-search.md`](superpowers/plans/2026-06-18-m6c-2-live-search.md).

> **Capability roadmap.** Phase 1 (M5 follow-up, done): core-op capabilities +
> per-document/world grants. Phase 2 (M6b): declarative, data-driven field-path
> capability requirements — server-authoritative, zero code-execution risk,
> covers the large majority of module rules. Phase 3 (separate later milestone,
> opt-in): **sandboxed** server-side validators for computed game-rule
> enforcement — its own threat model; never the default path.

### M7 · Layout-lite + theming scaffold ✅
> **DONE** (merged to local main; pushed at milestone completion). Delivered across
> M7a (server surface) · M7b (UI contribution architecture: server-mirrored
> contract declarations + client `ContributionRegistry` + Svelte `<Surface>`) · M7c
> (the SPA + `core-ui` shell + entry flow; the binary serves the embedded SPA) · M7d
> (icon-derived 3-tier SCSS theme + framework-neutral i18n seam + `ui_state`
> session-restore that returns you to your last world on reload). Specs/plans under
> `superpowers/`. Deferred within M7: multi-provider singleton conflict policy +
> capability version negotiation (`TODO.md`); `activeTab` restore + a tabbed sidebar
> (M11/M12, when there are multiple sidebar panels).
> Spec: [`superpowers/specs/2026-06-19-m7-layout-theming-design.md`](superpowers/specs/2026-06-19-m7-layout-theming-design.md).
> Decomposed into **M7a** (server surface), **M7b** (UI contribution
> architecture), **M7c** (shell + entry flow as modules + reactivity bridge),
> **M7d** (theming + i18n + session + tests) — each its own plan+execute cycle.

First Svelte 5 UI over the headless M6 core, built as a **UI-as-modules
contribution architecture**: every UI element (regions, panels, later combat
tracker / dice tray / HUDs) is a module contributing components into **surfaces**
(named string-contract mount points) declared by other modules, with
contract-based (`provides`/`requires`) dependencies resolved on the existing M6b
module system. Core owns contract resolution; the ui package hosts surfaces via a
framework-neutral `ui.surfaces` service (preserves whole-UI replacement).
- Full entry flow: first-run setup → login → world select → in-world table shell.
  Vite bundle replaces `src/server/static/`; `embed.rs` seam flips to `dist/`.
- Fixed VTT-standard region layout (top bar · tool rail · stage · sidebar ·
  status bar) provided by a first-party `core-ui` module; default panels are
  contributions. Stage is an M8 canvas placeholder.
- One dark theme (palette derived from `assets/icon`) via the 3-tier SCSS token
  system; i18n scaffold (`typesafe-i18n`, one `en` locale).
- Session state persisted in the DB: per-user opaque `ui_state` blob (server
  validates object+size-cap only; client owns structure).
- New server surface: `GET /worlds`, public `GET /api/config`, `GET/PUT
  /me/ui-state` + migration.
- The token set is proven against panel chrome and **explicitly re-audited when the first themed canvas overlays land (M8) and again when default-module sheets/browsers land (M12)** — the early set is not treated as final.
- **Pre-release framing:** no public release until ≥2 internal systems exercise
  the API (Phase 4 freeze gate), so the contribution API is built in full now and
  hardens through internal use — unfrozen, not third-party-stable. Deferred (no
  definable answer without a real second provider): multi-provider `singleton`
  conflict policy + capability version negotiation (logged in `TODO.md`;
  deterministic loud-fail placeholder until then).
- Excludes: drag-resize, pop-out / multi-window, multi-theme, user themes, module styling modes.

### M8 · ECS + scene rendering
- hecs hydration/mutation boundary; ECS→WS dispatch.
- Render-layer / filter abstraction spiked against the vision mask path first.
- Scene load, grid (square / hex), camera pan/zoom; basic token placement; measurement / template / drawing tools; pings.
- Minimal raw asset upload + static serving (scene backgrounds, token art), with **stable UUID asset identity from first upload** (links survive rename/move); no conversion / browsing / tagging — the full asset pipeline is Phase 2.
- Excludes: post-processing, multi-level maps, portals.

### M9 · Walls + vision + fog
- Vector walls as ECS components; movement blocking.
- Rust raycasting; per-player visibility polygons (`geo` union); PixiJS masks; persistent fog of war.
- GM vision mode. Server-authoritative geometric vision only (exempt from the optimistic path by design).
- Excludes: photometric / illumination coupling, darkvision / tremorsense / height, Web-Worker optimistic vision.

### M10 · Tokens
- Actor-linked tokens; shapes; instanced / unique modes; A* pathfinding with waypoints; status conditions; factions.

### M11 · Dice + chat
- From-scratch dice engine (notation, modifiers, advantage/disadvantage, DCs, success counting, tiers); hook integration; sequenced results.
- Chat log; whispers (user-to-user / GM-only).

### M12 · Minimal default modules
- Actor / scene browsers, generic actor / item sheets, chat panel — built against the public API, each treated as an API bug report.

### M12.5 · Backups + snapshot restore (gate precondition)
- Basic world backup (SQLite snapshot / per-world export) + restore path; minimal manual scheduling. Distinct from Phase-4 backup *automation*.
- Satisfies the dogfood-alpha gate's data-safety precondition.

**▶ Dogfood alpha gate** — backups (M12.5) must exist before real worlds accrue.

## Phase 2 — Full table
Combat tracker (initiative, hidden combatants, turn-event triggers; depends on M11 dice) → real asset pipeline (chunked upload, image conversion, tags, derived tags) + asset browser (regex / tag / dir search, preview / rename / move / tag) + bulk import/export → layout / theming completion (drag-resize, pop-out, multi / user themes, module styling modes) → vision / lighting completion (photometric, darkvision / tremorsense / height) → token enrichment (aura / light / sound / VFX emitters, trigger regions, token-art) → rollable tables (on the dice engine + document model), rich-text notes (on the document model), chat media linking (images; YouTube as thumbnail + external link only — no IFrame / Data API) → full default module suite → search consolidated into one milestone (single backend; no three-backend split).

## Phase 3 — Atmosphere
Audio (mixer, channels, playlists, world-clock sync; then spatial + wall occlusion; transcode via `symphonia` + `opus`/`vorbis_rs`) → VFX (sprite effects, concurrent SFX) → multi-level maps + portals → 3D dice (decide the rendering context up front: reuse the PixiJS WebGL context vs a separate three.js/WebGL + physics layer) → Discord audio-ducking module (OS audio-session monitoring — PipeWire / WASAPI / CoreAudio — never the proprietary Discord Game SDK; requires a dependency / licensing review before integration).

## Phase 4 — Platform & scale
Trusted local modding hardening → freeze the module API on evidence (≥1 external module ships without core patches, **or N internal modules across M independent systems exercise the full API surface** — whichever comes first, so the freeze is not deadlocked on an external author who may never appear) → [only if a marketplace is pursued] WASM sandbox + registry + signing / SRI / CSP + package browser → native wrappers (Tauri 2, Capacitor) → hardening + distribution (backup scheduling / automation, world snapshots, WS load + resync stress tests, rate limiting, rustls-acme TLS, Steam OpenID + plain-executable distribution).

## Cross-cutting (not deferred)
- Observability + desync telemetry: M4.
- Desync-convergence test: M4, maintained throughout.
- Backups: a basic backup + snapshot-restore deliverable (M12.5) satisfies the dogfood gate; Phase 4 adds scheduling / automation.
- Rate limiting on WS / upload: introduced with the surfaces it protects, not only at hardening.
- Error UX (disconnect, rejected optimistic op, failed upload): owned by M5 / M6 client work.
- Account model: self-host, admin-provisioned accounts (no self-registration / email in v1).
