# shadowcat — Architecture & Technology

Source of truth for the engine's structure, technology choices, and invariants. Decisions here are load-bearing; change them only with the scrutiny that established them. Paired with [`../PLAN.md`](../PLAN.md) (milestone roadmap).

## 1. System shape

shadowcat is a self-hostable virtual tabletop. Two halves, one shipped artifact:

- **Rust server** — authoritative state, persistence, networking, asset handling. Compiled by Cargo into a single native executable.
- **Browser client** — a framework-neutral "headless core" (state, networking, module loader, hook bus) plus a Svelte 5 default UI and a PixiJS canvas. Built by Vite into static assets.

The compiled client bundle is **embedded into the server binary** (`rust-embed`) and served over HTTP. Distribution is one executable; a browser is the only client runtime. Optional desktop (Tauri) and mobile (Capacitor) wrappers are post-MVP and reuse the same client against the embedded server.

Build-time toolchains never ship: Cargo builds the server, pnpm + Vite build the client, the result is one binary. A JavaScript package manager (pnpm) is a build-time dependency of the same class as Cargo — it produces static assets and is absent from the runtime.

Source resides under `src/`: `src/server/` (Rust workspace), `src/client/{core,ui,render}/` (headless core + Svelte default UI + engine-owned PixiJS render layer), `src/modules/` (first-party default modules), `src/types/` (generated TS types). The Rust server binary builds to `target/` (Cargo convention); the client bundle builds to `dist/`. Binary assets shipped with the app (e.g., the icon) live in `assets/` at the repo root, outside `src/`.

**Client UI packaging (realized — `PLAN.md` M8.5).** The browser client resolves into replaceable layers: a thin **app shell** (`@shadowcat/shell` — bootstrap, routing, session, the AppContext provider, the default-module-set wiring); a shared Svelte UI-runtime (`@shadowcat/ui-kit` — AppContext, the `<Surface>` host, the i18n adapter, the `sceneInteraction` bridge); a **swappable entry package** (`@shadowcat/module-entry` — login + world management, pre-world, behind a single `<Entry onEnterWorld>` contract) a self-hoster can replace to integrate external auth/identity; and **per-element in-game UI packages** under `src/modules/*` (`module-core-ui` owns the layout grid + region surfaces and contributes it into a singleton `root` surface; `module-{topbar,statusbar,stage,settings,assets,scene-tools}` each contribute one element) so any in-game element — or the whole layout — is independently moddable or replaceable. The shell assembles the default module set and passes it to the session as `modules: Module[]`. In-game elements communicate **only** through the seams (the M6b/M7 module + `provides`/`requires` contract system, `ContributionRegistry`, the surface host, AppContext, the engine-owned render-layer API) — never importing one another or the shell directly. First-party default modules are workspace packages compiled into the one bundle at build time; the dynamic loader path remains for untrusted third-party modules.

## 2. Invariants

These hold across every subsystem. Violating one is an architectural defect, not a tradeoff.

1. **Server-authoritative state.** The client sends *intents*; the server validates, applies, and broadcasts. No client is trusted for state, visibility, or permissions.
2. **Ordered, recoverable realtime.** Every broadcast carries a per-world monotonic sequence number from an atomic counter. Clients detect gaps and resync from a time-bounded event buffer, or a full snapshot. A server time source + client offset calibration exists in the networking layer before it has a consumer, so later wall-clock sync (audio, combat) is not a retrofit.
3. **Optimistic with rollback.** Clients may apply an intent locally for responsiveness, tagged with an intent id; the server's confirmation reconciles, and divergence rolls back to authoritative state. Vision recomputation is exempt in v1 — it is server-authoritative without client prediction by design (see `PLAN.md` M9).
4. **Permissions enforced server-side, per recipient.** One `PermissionContext` per connection gates reads/writes and filters every broadcast individually — hidden fields are stripped before transmission by default. The role model spans server roles (admin), world roles (GM / player / spectator), and document roles (owner / observer / none). Stripping is the default, not an absolute — see invariant 11 for when sent-then-hidden is the right call.
11. **User experience outranks data secrecy.** The player must never *see* what they shouldn't; whether hidden data reaches the client is secondary. Cheat prevention in a GM-run game is a secondary priority to the audio/visual experience: where secrecy and fidelity conflict, fidelity wins and the data is sent-then-hidden, with the client's rendering (fog, redaction) as the gate. Prefer designs that keep both when the cost is comparable; never degrade the experience to keep data off the wire. Two exceptions are ironclad and never traded: personally identifiable information, and security against remotely compromising another user's device.
5. **Documents are the source of truth; runtime state is derived.** Persistent data is a typed envelope (id, type, owner, permissions, `schema_version`, plus a display **`name`** — see invariant 6) carrying two content bodies, **`engine`** and **`system`**. Scene/runtime state (ECS) is hydrated from documents and is ephemeral.
6. **Three-band document shape: envelope name, typed `engine`, opaque `system` (M13-0).** Every document carries an envelope-level `name: Option<String>` (a universal display name, redacts to `null` under a per-property override — never stripped, so the envelope shape stays stable for every recipient) alongside two content bodies with different authority models:
   - **`engine`** — present iff `doc_type` is one of the 23 engine-defined types (tokens, actors, scenes, walls, regions, lights, drawings, templates, messages, world/vision/lighting/chat/dice config-docs, the faction/condition/channel registries, the world's system-declared defaults (`system-defaults`), and the combat family: `combat`, `combatant`, `resource-registry`, `effect`, `combat-history`). Typed, ts-rs-generated Rust structs (`src/server/src/data/engine/`) with **strict ingress validation**: `validate_engine`/`validate_engine_tree` reject an unknown field, a wrong-typed field, an engine body on a non-engine `doc_type`, or a missing `engine` body on an engine `doc_type` — checked on every Create/Update post-image, including embedded children, at one recursive chokepoint. This is the band engine-owned geometry (walls, regions, vision, tokens, movement) now lives in, replacing the pre-M13-0 practice of that geometry sharing the opaque `system` root.
   - **`system`** — unchanged opaque, system-defined JSONB body. The server's authority over it stays **structural only** — size caps, JSON validity, `deny_unknown_fields`, and permissions; it performs **no semantic/mechanical validation** and never interprets its content. This is the band game-system modules own exclusively.
   - **Module and system logic runs on the client, GM-authoritative** for the `system` band: the server is relay + persistence + structural validation, runs no third-party code in v1, and GM-originated intents carrying module-computed `system` state are accepted as authoritative on that basis (the cooperative-play trust model — install-time trust: only a GM activates modules). The `engine` band is NOT part of this trust exception — it is server-validated engine territory, not module-owned, so the "no semantic validation" carve-out applies to `system` only. Movement-collision and per-player vision (M9) read `engine` geometry directly rather than needing a `system`-body exception, since that geometry now lives in the typed, server-validated band by construction.
   - **The movement-collision gate scopes to `Operation::Update` only, by design.** `Room::publish`'s wall/vision-restriction gate (M9a/M10e-4) inspects only `Operation::Update` (a token move); `Operation::Create` (initial token placement) is intentionally ungated, since the create capability is already a privileged grant (GM or a place-token tool) and unrestricted initial placement is normal authoring behavior. This is not a movement-restriction bypass — placement and movement are different operations with different privilege gates.
7. **The public module API is framework-neutral; the UI is extendable and replaceable.** UI *extension* is via DOM / web-component mount points; logic via plain-TS hooks and services. The **headless core** (document store, hook bus, module loader) is a **Svelte-free TS module** — no Svelte runtime in its dependency closure — so a module on any framework (React / Vue / vanilla) consumes it without transitively pulling Svelte. A module may **extend** (mount points / slots), **replace** the default UI wholesale (panels, the application shell, canvas overlays), but **cannot** replace the PixiJS canvas host itself — the renderer is engine-owned and modules draw into it through the render-layer API. The Svelte core never leaks into the public surface.
8. **Mutations flow through an undoable boundary.** Document and ECS mutations are expressed as discrete, reversible operations (command/event records) from the start. This reversible representation is the **single shared substrate** for both optimistic rollback (reverting unconfirmed local speculation when authoritative state arrives) and undo (reverting confirmed, committed operations) — two distinct triggers and execution paths over one representation. No undo UI ships in v1, but the boundary supports it without a later rewrite.
9. **Permissive licenses only.** MIT / Apache-2.0 / BSD / zlib / MPL-2.0. No GPL / AGPL / SSPL / proprietary in the runtime or required toolchain. Media codecs must be royalty-free.
10. **Cross-platform from day one.** The server binary builds and is tested on macOS, Linux, and Windows; the browser client renders correctly on desktop **and** mobile (Android / iOS). This is a CI-verified build-time invariant, not a later port: paths use `std::path` (never hardcoded separators), OS-specific code is `#[cfg]`-gated for every target, CI runs a three-OS matrix, and every served page is responsive and touch-ready. Native wrappers (Tauri / Capacitor, §4) reuse the same artifacts and add no platform that the client does not already support.

## 3. Core technology (v1)

| Concern | Choice | License | Roll/Vendor | Rationale |
|---|---|---|---|---|
| Async runtime | tokio 1.52 | MIT | Vendor | Standard; lowest-risk dependency in the stack. |
| HTTP + WebSocket | axum 0.8 | MIT | Vendor | tokio-native, multi-maintainer; routing + WS in one crate. |
| Database | SQLite (JSONB 3.45+, FTS5) | Public domain | Vendor | Only option delivering single-binary, server-less self-host. Postgres deferred behind a `Repository` trait. |
| DB access | sqlx (sqlite feature) | Apache-2.0/MIT | Vendor | Compile-time-checked queries; keeps the Postgres door open behind the trait. `rusqlite` is the fallback if sqlx maintenance degrades. |
| Realtime protocol | custom event bus | — | Roll | Sequence numbers, per-world rooms, intent/confirm — domain logic. |
| Scene simulation | hecs 0.11 + custom exec/persistence | MIT/Apache | Vendor + Roll | hecs for storage/queries (compositional fit for token emitters); the async execution and document↔ECS boundary are ours. |
| Auth | argon2 + tower-sessions | MIT/Apache | Vendor | Password hashing; DB-backed sessions; admin-provisioned accounts (no self-registration/email in v1); server / GM / player / spectator + document observer roles. |
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
| Asset browser (regex/tag/dir search, preview/rename/move/tag) | asset pipeline + `Core.search` | Phase 2. |
| Rollable tables | dice engine + document model | Phase 2. |
| Rich-text notes | document model / `system` body | Phase 2. |
| Chat media linking (images, YouTube) | chat | Phase 2. YouTube = thumbnail + external link only (no IFrame/Data API), keeping the stack permissive. |
| Audio mixer (Web Audio + `standardized-audio-context`) | event bus | Phase 3. Simple play/stop/loop/volume first; spatial/occlusion later. |
| 3D dice | dice engine + a rendering-context decision | Phase 3. Decide up front: reuse the PixiJS WebGL context vs a separate three.js/WebGL + physics layer. |
| Discord audio ducking | audio mixer hook points; secondary module | Phase 3+. OS audio-session monitoring (PipeWire / WASAPI / CoreAudio) — never the proprietary Discord Game SDK; requires a dependency/licensing review before integration. |
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
- **Discord Game SDK** — proprietary. Discord audio ducking (deferred) is implemented via OS audio-session APIs, never the SDK.
- **specta / tauri-specta** — stuck in multi-year RC; `ts-rs` 12 is stable and maintained.
- **Pure-Rust/WASM frontend** — would discard PixiJS and eliminate first-class UI moddability (modders would have to write Rust).

## 6. Data model & validation

- **Envelope + typed `engine` + opaque `system` (M13-0).** A document is a typed Rust envelope (id, type, owner, permissions, `schema_version`, display `name`) plus two content bodies: a typed, server-validated `engine` JSONB body (present only for the 23 engine-defined doc types, including `system-defaults` and the combat family `combat`/`combatant`/`resource-registry`/`effect`/`combat-history` — see invariant 6) and an opaque `system` JSONB body the engine never interprets. Systems define the `system` body's meaning; the `engine` body's meaning is fixed by the engine's own typed Rust structs.
- **Two reserved directories inside `system`, universal location + singleton-system premise (M13b).** The engine reserves exactly two subtrees of the opaque `system` body for any game system to populate — `system.stats` (the variables directory: a `Record<key, Stat>` map any formula/reference dereferences into) and `system.mechanics` (the non-variable model directory: a system's own bucket/modifier/state shape). The reservation is location-only — entry shape inside each directory stays system-defined — and it holds for exactly **one active system per world** (a manifest `system: true` convention, enforced when a second system exists to conflict with the first; the world's currently-installed system owns both directories outright). The server's authority over both directories is still structural only per invariant 6 — size caps, `deny_unknown_fields`, permissions — schema-level enforcement of a specific system's shape is a subtree-scoped, data-driven server registry (deferred, `PLAN.md` M13f), not a special case of this reservation. Why two directories rather than one namespace, and why location-only: a single namespace holding both (`system.<system-id>.stats`, or a bare undifferentiated `system` body) mixes dereferenceable variables with model data, and a namespace branded with the system's own id couples every stat consumer to the identity of whichever system is active — a sheet, a formula reference or a modifier would have to name the system to find a stat. The two-location split removes both couplings at once, and branding buys nothing because the system role is singleton per world. The reservation stops at location because entry SHAPE is the system's to define: fixing an entry shape here would put engine opinion inside the band invariant 6 makes system-owned, and the engine has no basis for one. An optional progressive contract behind it (engine features reading any entry that happens to be `{current, max}`-shaped) remains open, since it adds a capability without constraining anything. A system's per-document version belongs inside the mechanics directory rather than the envelope because documents travel between worlds and system versions, so a copy must self-describe. The expression library stays path-agnostic throughout — references resolve through a consumer-supplied resolver — so this convention binds the SYSTEM's resolver, never the library.
- **Copy independence.** Actors and items exist independently across compendium, world, and embedded copies: modifying a world copy never alters the compendium template, and an embedded copy is independent of the document it was instantiated from.
- **Stable asset identity.** Assets are referenced by stable UUID from first upload, so moving or renaming an asset never breaks links — independent of when the browsing/conversion pipeline lands.
- **Schema migration: mechanism now, migrations later.** Documents carry `schema_version` and the data-model layer exposes a synchronous, client-side `migrateData` seam (coerce a document from its stored version to current on load/update — pure transform, no sandbox). Because nothing ships before v1, there are **no documents in existence to migrate**: v1 builds the migration *machinery* and the seam runs as a no-op pass-through, but **no actual migrations are authored** until a post-ship schema change creates the first real use case. Arbitrary bulk fix-up *scripts* are a separate, far-future concern.
- **Validation at boundaries.** The client validates the `system` body against the system's Zod schema before writes; the server enforces structural limits (size caps, field-path validity, `deny_unknown_fields`) and permissions on `system`, but never its semantic correctness (invariant 6). The `engine` body gets a stricter boundary: server-side shape/type ingress validation (`validate_engine`/`validate_engine_tree`, `deny_unknown_fields` on every engine struct) rejects a malformed body outright rather than merely capping its size — engine-owned geometry (movement-collision, vision) reads this typed, pre-validated band directly (invariant 6), no separate exception needed. Derived values are computed, never stored.
- **Settings resolve through a four-tier chain: engine → system-defaults → world → scene.** The
  engine ships a hardcoded fallback for every world-configurable setting (scene defaults,
  pathfinding, animation, combat); a `system-defaults` singleton document lets the world's active
  game-system module (the `SYSTEM_CONTRACT` winner, via its declared `Module.systemDefaults`,
  upserted idempotently by the GM's client on join) override those fallbacks per world; a
  `world-settings` document lets the GM override the same keys further; and a per-scene override
  is the narrowest and wins last. Every resolver in this chain (`resolve_combat_rules` server-side,
  `resolveSettingProvenance` client-side) walks the same four tiers in the same order — a resolver
  that stopped at three tiers, or reordered them, would silently disagree with the other about
  which layer supplied a given value.
- **The combat clock is a server-owned state transition, never a client-authored write.** A
  combat's turn order, round/turn counters, and turn-history log are mutated only by the server's
  own pure `transition` functions, dispatched from a fixed set of typed intents
  (`combat::handle_combat_intent`) and committed as a single command tagged
  `WriteOrigin::CombatTransition` — a tag the wire protocol has no way to construct, so a client
  can never forge a combat-clock write by hand-authoring a document `Update`. The per-turn
  movement-budget gate this gives the move executor is committed as a *separate* command from the
  token's position write, under a *different* origin (`WriteOrigin::Client`), specifically so that
  `CombatTransition`'s relaxed ownership check on the budget decrement can never be reused to
  authorize a move against a token the caller does not own.

## 7. Rendering provenance

Rendering and visibility techniques (raycast visibility polygons, fog of war, illumination) are implemented from **public sources only** — computational-geometry literature and public technique descriptions. No proprietary VTT or game-engine source is ingested, and no proprietary engine/product names appear in code. Public *documentation* and observable behavior of existing tools may inform the data and authority model; their source code may not.

## 8. Settled & open items

**Settled:** source layout is under `src/` (see §1); v1 accounts are admin-provisioned, no self-registration/email (see §3). The empty `source/` directory is renamed to `src/` at M1.

- **Per-milestone feature boundaries** are finalized in implementation plans, not here.
