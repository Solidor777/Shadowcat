# shadowcat — Architecture & Technology

Source of truth for the engine's structure, technology choices, and invariants. Decisions here are load-bearing; change them only with the scrutiny that established them. Paired with [`../PLAN.md`](../PLAN.md) (milestone roadmap).

## 1. System shape

shadowcat is a self-hostable virtual tabletop. Two halves, one shipped artifact:

- **Rust server** — authoritative state, persistence, networking, asset handling. Compiled by Cargo into a single native executable.
- **Browser client** — a framework-neutral "headless core" (state, networking, module loader, hook bus) plus a Svelte 5 default UI and a PixiJS canvas. Built by Vite into static assets.

The compiled client bundle is **embedded into the server binary** (`rust-embed`) and served over HTTP. Distribution is one executable; a browser is the only client runtime. Optional desktop (Tauri) and mobile (Capacitor) wrappers are post-MVP and reuse the same client against the embedded server.

Build-time toolchains never ship: Cargo builds the server, pnpm + Vite build the client, the result is one binary. A JavaScript package manager (pnpm) is a build-time dependency of the same class as Cargo — it produces static assets and is absent from the runtime.

## 2. Invariants

These hold across every subsystem. Violating one is an architectural defect, not a tradeoff.

1. **Server-authoritative state.** The client sends *intents*; the server validates, applies, and broadcasts. No client is trusted for state, visibility, or permissions.
2. **Ordered, recoverable realtime.** Every broadcast carries a per-world monotonic sequence number from an atomic counter. Clients detect gaps and resync from a time-bounded event buffer, or a full snapshot. A server time source + client offset calibration exists in the networking layer before it has a consumer, so later wall-clock sync (audio, combat) is not a retrofit.
3. **Optimistic with rollback.** Clients may apply an intent locally for responsiveness, tagged with an intent id; the server's confirmation reconciles, and divergence rolls back to authoritative state.
4. **Permissions enforced server-side, per recipient.** One `PermissionContext` per connection gates reads/writes and filters every broadcast individually — hidden fields are stripped before transmission, never sent-then-hidden.
5. **Documents are the source of truth; runtime state is derived.** Persistent data is a typed envelope plus an opaque, system-defined `system` body. Scene/runtime state (ECS) is hydrated from documents and is ephemeral.
6. **Module and system logic runs on the client, GM-authoritative.** The server is relay + persistence + structural validation; it runs no third-party code in v1. This preserves the single-binary story and matches the cooperative-play trust model (install-time trust: only a GM activates modules).
7. **The public module API is framework-neutral.** UI extension is via DOM / web-component mount points; logic via plain-TS hooks and services. The Svelte core never leaks into the public surface — modders use any framework.
8. **Mutations flow through an undoable boundary.** Document and ECS mutations are expressed as discrete, reversible operations (command/event records) from the start. No undo UI ships in v1, but the boundary supports undo without a later rewrite.
9. **Permissive licenses only.** MIT / Apache-2.0 / BSD / zlib / MPL-2.0. No GPL / AGPL / SSPL / proprietary in the runtime or required toolchain. Media codecs must be royalty-free.

## 3. Core technology (v1)

| Concern | Choice | License | Roll/Vendor | Rationale |
|---|---|---|---|---|
| Async runtime | tokio 1.52 | MIT | Vendor | Standard; lowest-risk dependency in the stack. |
| HTTP + WebSocket | axum 0.8 | MIT | Vendor | tokio-native, multi-maintainer; routing + WS in one crate. |
| Database | SQLite (JSONB 3.45+, FTS5) | Public domain | Vendor | Only option delivering single-binary, server-less self-host. Postgres deferred behind a `Repository` trait. |
| DB access | sqlx (sqlite feature) | Apache-2.0/MIT | Vendor | Compile-time-checked queries; keeps the Postgres door open behind the trait. `rusqlite` is the fallback if sqlx maintenance degrades. |
| Realtime protocol | custom event bus | — | Roll | Sequence numbers, per-world rooms, intent/confirm — domain logic. |
| Scene simulation | hecs 0.11 + custom exec/persistence | MIT/Apache | Vendor + Roll | hecs for storage/queries (compositional fit for token emitters); the async execution and document↔ECS boundary are ours. |
| Auth | argon2 + tower-sessions | MIT/Apache | Vendor | Password hashing; DB-backed sessions. |
| Permissions | custom `PermissionContext` | — | Roll | Per-recipient broadcast filtering + property-level stripping is domain-specific. |
| Validation / types | Zod v4 (`zod/mini`), ts-rs 12, Serde `deny_unknown_fields` | MIT/Apache | Vendor | Client-side schema validation; Rust→TS type generation; unknown fields rejected at both ends. |
| Dice | custom TS engine | — | Roll | Core mechanic; full control over notation, hooks, broadcast. |
| Vision geometry | `geo` | MIT/Apache | Vendor | Polygon boolean ops (visibility-polygon union). |
| Client embedding | `rust-embed` | MIT | Vendor | Bakes the built client bundle into the binary. |
| UI framework | Svelte 5 (runes) | MIT | Vendor | Compiled, lean output; default UI only — modders use any framework. |
| Canvas renderer | PixiJS v8 | MIT | Vendor | Mature WebGL 2D: sprite batching, filter pipeline, mask compositing. Rebuilding this is the largest avoidable cost in the project. |
| Build tooling | Cargo, Vite, pnpm | MIT | Vendor | pnpm is build-time only; output embeds into the binary. |

## 4. Deferred behind abstractions

Each item is *designed for* now (the seam exists) and *built* only when its trigger fires.

| Deferred | Seam in place | Build trigger |
|---|---|---|
| PostgreSQL | `Repository` trait | A real multi-tenant / many-concurrent-world hosted deployment. |
| Full-text search engine (Tantivy) | `Core.search` API over FTS5 | FTS5 relevance/scale becomes inadequate (large compendium libraries, BM25 tuning, faceting). |
| Asset conversion — images (`image` 0.25 + `webp`/libwebp), audio (`symphonia` + `opus`/`vorbis_rs`) | raw upload + static serving; asset pipeline | Phase 2 (images) / Phase 3 (audio). v1 stores and serves uploads unconverted. No FFmpeg; all replacements are royalty-free. |
| Audio mixer (Web Audio + `standardized-audio-context`) | event bus | Phase 3. Simple play/stop/loop/volume first; spatial/occlusion later. |
| VFX, post-processing, photometric lighting, advanced vision modes, multi-level maps/portals | render-layer abstraction; ECS components | Phase 2–3, after the gameplay loop is proven. |
| Undo/redo UI | undoable mutation boundary (invariant 8) | When users need it; no engine change required. |
| Server-side untrusted execution (sandbox) | client-side GM-authoritative model | Only if a marketplace with untrusted authors is pursued — then WASM (wasmtime/extism) or rquickjs, never Deno. |
| Module registry / signing / SRI / CSP | local trusted-module loading | Same marketplace trigger. |
| Compression (app-level `zstd`), content hashing (blake3, differential sync) | — | When profiling shows storage/transfer cost matters. |
| Native wrappers (Tauri 2, Capacitor) | embedded-server client | After the web app is feature-complete. |

## 5. Explicitly rejected

- **Bun / Node as a server runtime** — pure-Rust server; no JS on the server.
- **PostgreSQL + SQLite in parallel from day one** — doubles the data layer (JSONB vs JSON, two FTS engines, two migration trees) to serve a scale tier v1 does not target.
- **Deno** — a ~100 MB V8 second runtime undercuts the single binary; its `--allow-*` model is a weak sandbox. Client-side GM-authoritative logic removes the need entirely.
- **FFmpeg as a hard dependency** — GPL contamination risk (libx264 etc.), LGPL static-link friction, and H.264/H.265/AAC patent exposure. Replaced by small royalty-free libraries.
- **Tantivy in v1** — a third, non-transactional storage system; FTS5 is crash-consistent (updates inside the row's transaction) and sufficient at VTT scale.
- **`steamworks` crate / Steam Rich Presence** — requires redistributing Valve's proprietary `steam_api`. Steam stays OpenID 2.0 auth + plain-executable distribution only.
- **specta / tauri-specta** — stuck in multi-year RC; `ts-rs` 12 is stable and maintained.
- **Pure-Rust/WASM frontend** — would discard PixiJS and eliminate first-class UI moddability (modders would have to write Rust).

## 6. Data model & validation

- **Envelope + opaque body.** A document is a typed Rust envelope (id, type, owner, permissions, `schema_version`) plus a `system` JSONB body the engine never interprets. Systems define the body's meaning.
- **Schema migration is client-side and synchronous.** A `migrateData` step coerces a document from its stored `schema_version` to current on load/update — a pure data transform in the data-model layer, no sandbox. Arbitrary bulk fix-up *scripts* are a separate, far-future concern.
- **Validation at boundaries.** The client validates the `system` body against the system's Zod schema before writes; the server enforces structural limits (size caps, field-path validity, `deny_unknown_fields`) and permissions. Derived values are computed, never stored.

## 7. Rendering provenance

Rendering and visibility techniques (raycast visibility polygons, fog of war, illumination) are implemented from **public sources only** — computational-geometry literature and public technique descriptions. No proprietary VTT or game-engine source is ingested, and no proprietary engine/product names appear in code. Public *documentation* and observable behavior of existing tools may inform the data and authority model; their source code may not.

## 8. Open items (require confirmation before settled)

- **Monorepo layout & source-dir naming.** `CLAUDE.md` states source resides in `src/`; the repo currently has an empty `source/`. A Rust + JS monorepo needs a server/client split. Proposed (pending consent): `server/`, `client/{core,ui}/`, `modules/`, `types/`, `docs/`. Confirm the root layout and resolve `src/` vs `source/`.
- **Account model.** v1 assumption: self-hosted, admin-creates-users, no email / password-reset flow. Confirm.
- **Per-milestone feature boundaries** are finalized in implementation plans, not here.
