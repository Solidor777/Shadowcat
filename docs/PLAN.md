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

### M8 · ECS + scene rendering ✅
> **DONE** (merged to main, pushed). M8a (server scene foundation: parent_id + per-world
> hecs read-model + SceneDerived egress) · M8b (raw asset upload/serve + panel) · M8c
> (client render foundation: layers/camera/grid/reconciler + render-layer/compositor API +
> identity vision-mask spike) · M8d (tokens + interaction): **M8d-1** token rendering +
> tween/ticker; **M8d-2** scene lifecycle (GM auto-create) + canvas tool API + `scene-tools`
> module + place/select/move (render-from-optimistic-view); **M8d-3a** drawing/template
> entities + draw/template tools (shape backend node + pure geometry + preview overlay);
> **M8d-3b** client-local measurement + pings (out-of-band `scene_ping` server broadcast +
> transient rings). Every slice buddy-checked. Specs/plans under `superpowers/`.
- hecs hydration/mutation boundary; ECS→WS dispatch.
- Render-layer / filter abstraction spiked against the vision mask path first.
- Scene load, grid (square / hex), camera pan/zoom; basic token placement; measurement / template / drawing tools; pings.
- Minimal raw asset upload + static serving (scene backgrounds, token art), with **stable UUID asset identity from first upload** (links survive rename/move); no conversion / browsing / tagging — the full asset pipeline is Phase 2.
- **Token rendering is forward-looking** (M8d ships static images only): tokens render as scene **sprites** — Container-based visuals, not raw images — behind a token-visual *source* abstraction that admits **multi-face, animated, and procedurally-generated** visuals later; **client-side tweening** toward document-authoritative transforms (ephemeral, never persisted/ECS); **fx** via the render-layer filter seam; **emotes** as transient overlays. A per-frame render ticker (animation/tween/fx) and a generalized `DisplayBackend` node API arrive with motion. M8 implements only static-image tokens; the architecture must not preclude the rest (full token features = M10). Detailed in the M8d spec.
- Excludes: post-processing, multi-level maps, portals.

### M8.5 · UI packaging decomposition ✅
> **COMPLETE** (spec `superpowers/specs/2026-06-24-m8.5-ui-packaging-decomposition-design.md`; decomposed a→b→c, each buddy-checked, zero behavior change). **M8.5a:** new shared `@shadowcat/ui-kit` Svelte-runtime package (appContext/Surface/i18n adapter/sceneInteraction + a `/test` fixture subpath) + renamed `@shadowcat/ui` → `@shadowcat/shell`. **M8.5b:** swappable `@shadowcat/module-entry` (single `<Entry>` component, `{onAuthenticated, onEnterWorld}` contract, internal setup→login→world-select step machine, co-located `entryApi`); shell boot split renders `<Entry>` for pre-world. **M8.5c:** split `core-ui` into per-element packages (`@shadowcat/module-{topbar,statusbar,stage,settings,assets}`) + `module-core-ui` reduced to the layout (owns `Layout` + `root`/region surfaces) + module-owned layout via a singleton `root` surface + Settings logout via a new `AppContext.logout` seam + asset-CRUD REST moved to `@shadowcat/core` (shared by assets + scene-tools) + `scene-tools` relocated + `WorldSession` opts → `modules: Module[]`. Every in-game element (and the layout) is now an independently replaceable `src/modules/*` package; first-party defaults compile into the one bundle, the dynamic loader path stays for third-party modules. Realizes the **client UI packaging** in [`design/ARCHITECTURE.md`](design/ARCHITECTURE.md) §1.
- Extract the **entry flow** (setup / login / world select / world management) into its own **swappable package** a self-hoster can replace to integrate external auth/identity (today plain views inside `@shadowcat/ui`).
- Split the first-party `core-ui` module into **per-element in-game packages** under `src/modules/*` (each region / panel / tool its own module), so each is independently moddable/replaceable.
- Separate the **thin app shell** (bootstrap, routing, session, surface host) from both entry and content.
- **Includes splitting today's monolithic entry views + `core-ui`** — not just greenfield. The contract-only element-boundary discipline (and new in-game UI shipping as `src/modules/*` packages) is adopted from M8d onward, so this milestone is mechanical extraction, not a redesign.
- Excludes: changing the contract/surface model itself (already built in M6b/M7).

### M9 · Walls + vision + fog ✅
> **COMPLETE** (merged + pushed). Cross-cutting spec `superpowers/specs/2026-06-22-m9-walls-vision-fog-design.md`
> (decisions locked), decomposed **M9a → M9b → M9c**. **M9a DONE** (merged + pushed): wall
> `doc_type` + render + wall tool; **server-authoritative movement-blocking** (a non-GM token move
> crossing a `blocksMove` wall is rejected before the write — the first server-side semantic
> geometry, a new ARCHITECTURE #6 exception; buddy-checked, a Critical post-image bypass fixed).
> **M9b DONE** (merged + pushed): clean-room visibility-polygon raycaster (angular sweep over
> `blocksSight` walls), per-recipient `vision` SceneDerived channel shipping scene-tagged polygons,
> engine-owned two-state fog mask (inverse-masked white-fill union — no `geo` dep). GM → `mode:"all"`;
> a player gets only their own polygons; a token-less player gets full fog. Two blind security
> reviews reconciled: fail-closed garbled payload, cross-scene scoping, ±π seam (see the plan's
> "Implementation deviations"). **M9c-1 DONE** (merged + pushed): persistent per-(scene,player)
> explored fog (`explored_fog` table + sparse cell set + dispatch-layer accumulation), a three-state
> fog shader (unexplored = darkest / explored = dimmed / visible = clear), and a GM see-all/preview
> toggle. Two blind security reviews (no Critical/Important; isolation + fail-closed + under-reveal
> race verified) — cell-scan cap, cleanup TODO, player wire test folded in. **M9c-2 DONE** (merged +
> pushed): **GM see-as-player** via a GM-authorized `SceneSubscribe{as_user}` (Approach B, chosen on
> the user durable/secure/performant directive over the spec "all masks to GM" variant — see §10 D-V3
> + the plan's decision #5). The server resolves the target role server-side (non-GM/non-member
> rejected), computes the `vision` payload as that player, and see-as is read-only; the client adds a
> unified GM vision dropdown. Two blind security reviews: BOTH PASS, no Critical/Important (the
> player-to-player `as_user` boundary is airtight) — dup-sub-id guard + stale-picker reset folded in.
- Vector walls as ECS components; movement blocking.
- Rust raycasting; per-player visibility polygons (`geo` union); PixiJS masks; persistent fog of war.
- GM vision mode. Server-authoritative geometric vision only (exempt from the optimistic path by design).
- Excludes: photometric / illumination coupling, darkvision / tremorsense / height, Web-Worker optimistic vision.

### Pre-M10 cleanup ✅
> Triaged `POST_WORK_FINDINGS.md` + `TODO.md` and closed every fixable item not blocked on unbuilt
> infra. 12 tasks: by-id routes 404-to-non-members; embedded-child size cap + GmOnly redaction; last-GM
> guard; asset-replace rate-limit; per-user ping limiter; deterministic broadcast-`Lagged` test; a
> two-axis capability model — `WorldCapDefaults {all, by_type, role_caps}` with doc_type-scoped
> per-document grants + a GM-configured `WorldRole` `core:create` gate (GM-only by default); members
> endpoint returns usernames + see-as picker labels by username; convergent offline-intent replay
> (predict + queue while reconnecting, FIFO flush after resync). Spec/plan in `docs/superpowers/`.
> Deferred (blocked on infra): merge engine, module management, M12 multi-scene, rotation authoring,
> world/scene deletion, `tower_sessions` sweep.

### M10 · Tokens
> **COMPLETE** — the built scope shipped through M10h; the two remaining visual-polish
> checkpoints, M10i (`generated`) and M10j (`fx` + emotes), are **deferred to Phase 2** by user
> decision (their seams already exist — see the M10h block below). Cross-cutting spec `superpowers/specs/2026-06-24-m10-tokens-design.md`
> (decisions locked), decomposed into 10 checkpoints **M10a–j** across 4 phases
> (plan per checkpoint; `/clear` between). **M10a DONE** (merged --no-ff to LOCAL main,
> NOT pushed — push gate = full M10): the game `Actor` doc + **linked** (`actor_id` +
> name/visual/size override whitelist) vs **instanced** (embedded copy + `source`
> provenance) tokens; the single `resolveTokenActor → EffectiveActor` read-through;
> `TokenView` visual resolution; the `ActorSelection` seam + place-tool actor stamping
> (link/instance per `prototype`, with a user-configurable keep-after-place toggle); the
> swappable `@shadowcat/module-actors` create/list/pick package; and the user-side
> `actor_role`→`user_role` rename (the game entity now owns the name "Actor").
> Buddy-checked (1 agreed deep-clone fix + 2 one-sided minors resolved). Plan:
> `superpowers/plans/2026-06-24-m10a-actor-model.md`.
>
> **M10b DONE** (merged --no-ff to LOCAL main, NOT pushed — push gate = full M10):
> **factions** — a world-scoped singleton `faction-registry` config-document (an id→faction
> **map**, so adds are single-key field-Updates; `set_pointer` cannot grow arrays), a
> replaceable `@shadowcat/module-factions` that seeds 3 GM defaults idempotently + the GM
> editor, faction-colored token borders (`TokenNodeSpec.borderColor`), and faction
> **group-select** (a `TokenSelection` seam + multi-drag + select-all-of-faction); **name
> privacy** — a new `OwnerOrGm` visibility tier (`Access::is_owner` + a single `can_see`
> predicate, so an owner sees `OwnerOrGm` but never `GmOnly`) honored on every egress path
> (whole-doc, update-delta, embedded, search, HTTP), **retroactive redaction** that nulls a
> now-hidden field for non-authorized recipients when a GM tightens permissions (old:null —
> no pre-image leak), the fail-closed `actorDisplayName` accessor, and the GM hide control.
> Buddy-checked (two blind reviewers, converged: 1 Important embedded-coverage finding fixed).
> Plan: `superpowers/plans/2026-06-24-m10b-factions-name-privacy.md`.
>
> **M10c DONE** (merged --no-ff to LOCAL main, NOT pushed — push gate = full M10):
> **conditions (markers only)** — a world-scoped singleton `condition-registry` config-document
> (id→`{name,icon}` **map**, same single-key-Update shape as factions), a replaceable
> `@shadowcat/module-conditions` that idempotently seeds a generic emoji set (GM) + the GM editor
> + a token-selection-driven **toggle palette**; actor-data `conditions: string[]` resolved via
> `resolveConditions` and rendered as upright emoji **badge** chips (`TokenNodeSpec.badges`);
> `conditionTarget` resolves the write site (linked → actor `/system/conditions`; instanced →
> token `/embedded/actor/0/system/conditions`); the GM-or-owner toggle is gated by a new advisory
> `AppContext.canEdit(doc, path)` (mirrors the server Update-path check via the `canWritePath`
> capability mirror; server stays authoritative). No mechanical effects (deferred to combat).
> Buddy-checked. Plan: `superpowers/plans/2026-06-24-m10c-conditions.md`.
>
> **M10d DONE** (merged --no-ff to LOCAL main `77a47ba`, NOT pushed — push gate = full M10):
> **shapes + footprint** — `shape: "square" | "circle"` field in
> `ActorSystem` + per-token override in `TokenOverrides` whitelist; `resolveTokenBox(token, store,
> eff?) -> TokenBox {x,y,w,h,shape}` as the single chokepoint for scene-pixel footprint (actor-
> backed: `EffectiveActor.size × grid cell`; raw/dangling: `token.system.w/h` + `"square"`;
> fail-closed); `footprintRadius(eff) -> number` (grid-unit bounding-disc radius seam for M10e+
> pathfinder); `TokenNodeSpec.shape` reconciler + ellipse border in `@shadowcat/render`; shape/
> size-aware `topTokenAt` hit-test (point-in-ellipse vs point-in-rect) + selection ring in
> `@shadowcat/module-scene-tools`; shape + size editing (create form + per-row GM inline editor)
> in `@shadowcat/module-actors`. Plan: `superpowers/plans/2026-06-24-m10d-shapes-footprint.md`.
>
> **M10e EXPANDED (design done):** what was a single "Pathfinding — grid A*" checkpoint grew,
> on user direction, into a **vision/lighting/movement** foundation, because the user's
> requested **movement restriction** (a player may only move a token into areas they can
> **see** / have **revealed** / **unrestricted**; GM unrestricted — to stop accidental map
> reveals) redefines "what a player can see" from pure line-of-sight to **LOS ∩ (lit ∨
> darkvision)**. New cross-cutting spec `superpowers/specs/2026-06-24-m10e-vision-lighting-
> movement-design.md` (approved) decomposes M10e into **6 sub-checkpoints**: **M10e-1** vision/
> lighting data model + config · **M10e-2** server lighting-aware vision (per-(user,scene) grid
> visibility mask; the secrecy gate) · **M10e-3** client lighting render · **M10e-4** movement
> restriction at the M9 `Room::publish` gate · **M10e-5** movement animation (speed + easing) ·
> **M10e-6** grid A* pathfinder (consumes the mask). Order e-1→e-2→{e-3,e-4}→e-6; e-5 anytime.
> Scene axes (world-default + per-scene override): LOS-restriction, lighting-enabled (master),
> light-mode (global-illumination | environment-light), fog, per-actor vision modes (darkvision);
> environment light = edge-projected, occludable by a new `blocksLight` wall flag, color+intensity
> for day/night (module-automatable). **M10e-1 DONE** (vision/lighting/movement **data model V1**,
> client-only, zero Rust): config-docs `world-settings`/`light-gradation`/`vision-modes` +
> resolvers + per-scene vision/lighting overrides (`grid.distance`) + `light` doc_type + wall
> `blocksLight` + `EffectiveActor.visionModes` + new `@shadowcat/module-game-settings` (GM seed +
> world/scene/gradation/vision-mode editors, inherit = `null`) + actor darkvision authoring.
> SDD-executed (9 tasks, per-task two-reviewer gate + whole-branch buddy-check CONVERGED PASS);
> merged --no-ff to LOCAL main; full client gate green. **M10e-2 DONE** (server lighting-aware
> vision, Rust): `scene/lighting.rs` (pure illumination — gradation bands, light falloff, per-cell
> max-compose with `blocksLight` occlusion) + `SceneEcs` config-doc/actor side-tables + fail-closed
> server resolvers (mirror scene-docs.ts + actor.ts `resolveTokenActor`) + `player_lit_mask` (the
> per-(user,scene) `LOS ∩ (lit ∨ darkvision)` secrecy gate, fail-closed) + additive `lit` vision
> payload (`{mode, polygons, bands, lit}`; GM stays `mode:"all"`) + room cold-start hydration.
> SDD-executed (10 tasks, per-task two-reviewer gate + whole-branch buddy-check CONVERGED PASS; a
> Critical caught — `all_bright` left players blind — plus a precedence inversion vs
> `resolveTokenActor` and a cell-span overflow DoS, all fixed); merged --no-ff to LOCAL main; server
> gate green. Deviation (logged in TODO): environment light is flat ambient, not edge-projected,
> until scenes gain dimensions (placed-light occlusion IS implemented). **M10e-3 DONE** (client
> lighting render): faithful per-cell darkvision `renderHint` threaded through the server vision
> frame (`VisionMode.render_hint`, `player_lit_mask` highest-floor-wins per-cell hint resolve); wire
> `vision` payload extended to 5-int cells `[i,j,band,tint,hint_idx]` + top-level
> `renderHints:[String]` table; client `Lighting` class (`src/client/render/src/lighting.ts`:
> band→darkening alpha + tint + desaturate hint + day/night interpolation); engine-owned `lighting`
> core layer (CORE_LAYERS index 7, between `templates` and `mask`); `PixiBackend.setLighting`
> (per-cell darkening/tint + `BlurFilter` soft edges, gray-wash desaturate approximation). Lighting
> is COSMETIC — fog stays the secrecy gate; hint never widens visibility. Two deferrals logged to
> `POST_WORK_FINDINGS.md` (blur-not-gradients + desaturate overlay approximation).
> Plan: `docs/superpowers/plans/2026-06-25-m10e-3-client-lighting-render.md`.
> **M10e-4 DONE** (movement restriction): server-authoritative gate at the M9 `Room::publish`
> chokepoint. A non-GM token move whose supercover cells aren't all inside the user's visibility
> mask is rejected (`DataError::Forbidden`, before the write, no seq) — `visible` (current mask) /
> `revealed` (mask ∪ `get_explored`) / `unrestricted` (walls only); GM exempt; entire-move
> (supercover) not just endpoint; `partialCellLeniency` selects strict(center) vs lenient(corner)
> rasterization. New `scene/movement.rs` `supercover_cells` (DoS-capped, fail-closed); `visible_cells`
> gate mask reuses the egress `player_lit_mask` primitives (`cell_visible`/`lighting_inputs`/
> `source_los_poly`/`point_qualifies`) so the gate mask **equals** the egress secrecy mask (spec §13,
> parity-tested across env/global-illumination/darkvision/LOS+wall); `get_explored` lifted to the
> `Repository` trait. SDD-executed (5 tasks, per-task two-reviewer gate + whole-branch buddy-check
> CONVERGED PASS, zero Critical/Important); merged --no-ff to LOCAL main; full server suite green.
> Plan: `docs/superpowers/plans/2026-06-25-m10e-4-movement-restriction.md`.
> **M10e-6 DONE** (grid A* pathfinder): server-authoritative pure grid A* in
> `scene/pathfinding.rs` (`DiagonalRule` + `resolved_diagonal_rule` world-only resolver;
> `PathGrid`; `cell_enterable` — full geometric footprint-disc clearance vs `blocksMove` walls
> + ALL footprint cells in the non-GM mask + center-step; `astar_leg` — king-moves, 4 diagonal
> rules, 5-10-5 parity tracked in the `(cell,parity)` node and carried across waypoint legs,
> admissible+consistent heuristics, stale-pop skip, `MAX_PATH_NODES`/`MAX_WAYPOINTS`/
> `MAX_FOOTPRINT_CELLS` fail-closed bounds; `find` — validation, search window AABB+8-cell
> margin, parity carry across legs, cost sum, cell-center output). `SceneEcs::pathfind` reuses
> the SAME `visible_cells` mask as the M10e-4 movement gate (spec §13 — never fork the per-cell
> visibility decision; route ⊆ gate-allowed by construction); unions `explored`
> (`ExploredSet::iter`) for `revealed`; GM unconstrained; empty non-GM mask ⇒ Unreachable
> (fail-closed). New `move_walls(scene)` accessor (the `blocksMove` segments). `Pathfind`/
> `PathResult`/`PathError` one-shot wire frames (to the requesting connection only; `get_explored`
> fetched off the scene read lock — no lock across await). Client: `WsClient.pathfind` +
> `AppContext.pathfind` correlated-request seam (via `WorldSession` + `Table.svelte`); measure-tool
> route mode with path-preview overlay + movement-budget readout; ruler `Grid.distance()` gains
> the `alternating` (5-10-5) rule wired from `resolveSceneSettings(...).diagonalRule` into the
> `Stage GridSpec`. `cost_field` accepted but inert (uniform weight=1; activates in M10g). SDD-
> executed (11 tasks, per-task two-reviewer gate + whole-branch buddy-check CONVERGED PASS, zero
> Critical/Important); merged --no-ff to LOCAL main.
> Plan: `docs/superpowers/plans/2026-06-25-m10e-6-grid-pathfinder.md`.
> Spec: `docs/superpowers/specs/2026-06-25-m10e-6-grid-pathfinder-design.md`.
>
> **M10e-5 REDIRECTED → server-authoritative movement model.** The M10e-5 animation engine was
> built (duration/easing/interruptible `TokenAnimator`, Stage config wiring — KEPT), but its
> optimistic client-chained route-commit was DROPPED: a buddy-check exposed that optimistic
> prediction of *gated* moves rubber-bands, so the model was redirected to **server-authoritative
> gated moves** (request-only, server-executed, atomic state, moving-lock, vision-gated,
> region-arrestable). Decomposed **M1** (server move execution + mover-only render-path) → **M2**
> (observer render-path + continuous client vision) → **M3** (vision-gated pathfinder + region
> hook). Spec: `docs/superpowers/specs/2026-06-25-server-authoritative-movement-design.md`.
>
> **M1 DONE** (branch `m10e-5-movement-animation`, commits `98bf191..15076ca`, all green, NOT
> pushed/merged — push gate = full M10): `MoveRequest`/`MoveExecuted`/`MoveError` protocol; pure
> `scene/move_exec.rs` executor (per-step walls + vision-mask + region-arrest hook; §13 per-cell
> mask-parity with the `publish` gate, no fork; stricter on path-shape via king-step adjacency;
> new `token_position` + `resolved_animation_speed`); `commit_ops_locked` (gate-free `publish`
> tail) + `Room::execute_move` (`publish_guard` held across the whole validate→commit = atomic;
> Revealed = `visible_cells ∪ explored`; `moving` lazy-expiry lock; OCC pre-image defense-in-depth;
> GM wall-honored, diverging from `publish`'s legacy GM wall-bypass); `conn.rs handle_move_request`
> (mover-only `etx` reply, generic `MoveError` — no geometry leak); client `WsClient.moveRequest` +
> `AppContext.moveRequest` + request-only measure-tool route-commit (the M10e-5 animator drives the
> returned render-path; `collinearRuns` + `path-runs.ts` removed). SDD-executed (8 tasks, per-task
> two-reviewer gate + whole-branch buddy-check scoped Tasks 2,3,4 CONVERGED — 1 Critical refuted by
> ground truth, 5 Minors fixed; reviewed skill-update gate PASS).
> Plan: `docs/superpowers/plans/2026-06-25-m1-server-authoritative-move-execution.md`.
>
> **M2 DONE** (branch `m10e-5-movement-animation`, commits `f403ff1..d748219`, all green, NOT
> pushed/merged — push gate = full M10): streamed continuous vision, server-precomputed and
> strictly leak-free. `PosSample`/`VisionSample`/`ServerMsg::MoveStream` protocol (ts-rs + Zod
> mirror); `scene/move_stream.rs` pure path sampler (`sample_path`, arc-length parameterization,
> `MAX_VISION_SAMPLES`=96 shared cap); `SceneEcs::player_vision_inputs`/`VisionMoveInputs::polygons_at`
> (mover vision trajectory — full-wall-set raycast per path sample, reusing `sight_walls` +
> `vision::visibility_polygon`, no new vision model); `conn.rs egress_loop`'s dedicated `MoveStream`
> branch (`clip_move_stream`/`observer_vision_polys_for_scene`) — THE secrecy boundary: mover gets
> the full trajectory + `mover_vision`, an observer gets only the samples their OWN authoritative
> vision admits with `mover_vision` nulled, a wholly-occluded move is suppressed (zero frames, not
> an empty-`samples` frame); client `WsClient.onMoveStream` broadcast-driven playback (`MoveExecuted`
> fully retired) + `TokenAnimator.animateSamples` (time-synced tween, gap/occlusion detection,
> catch-up) + engine `visionSweeps` fog-sweep (snap, then `fog-blend.ts`/`setVisibilityBlend`
> render-texture cross-fade) + `worldSession`'s active-scene filter on `onMoveStream` (cross-scene
> leak guard). SDD-executed (8 tasks, per-task two-reviewer gate; reviewed skill-update gate DONE:
> scene-rendering, realtime-sync, client-shell). Whole-branch buddy-check (2 independent blind
> opus reviewers) CONVERGED: no-leak/§13-parity/no-lock-across-await/determinism all confirmed;
> 1 Important (client-side backward-extrapolation on leading-occlusion clips) fixed + reverified.
> Known v1 limitation (by design, not a bug): live
> cross-animation concurrency deferred (`docs/TODO.md`) — a move's per-recipient clip is computed
> once at its execute time, so two simultaneous moves don't reveal each other mid-walk if a
> watcher's vision opens after the clip; reconciles at the stop + next `vision` rebroadcast.
> Plan: `docs/superpowers/plans/2026-06-25-m2-streamed-continuous-vision.md`.
> Spec: `docs/superpowers/specs/2026-06-25-m2-streamed-continuous-vision-design.md`.
>
> **M3 DONE** (branch `m10e-5-movement-animation`, commits `7043419..fb8b7dd`): closes buddy-check
> P1 at the root by making the M10e-6 grid-A* router's vision-mask predicate a superset of the M1
> move executor's — `cell_enterable` now unions `movement::supercover_cells(from, to, cell)` (the
> same primitive `move_exec.rs`/`ws/room.rs::publish` use per step, including diagonal
> corner-flankers) into its mask check alongside the existing footprint-disc test, and fails closed
> on a degenerate/over-cap `None` result exactly like the gate. Restores `route ⊆ gate-allowed` for
> the sub-0.5-cell-footprint diagonal case the P1 buddy-check exposed. Also adds a same-shaped inert
> region-arrest hook (`fn region_arrests(_to: Cell) -> bool { false }`) to the router, mirroring
> `move_exec.rs`'s M1 stub, so M10g wires real region data into one hook shape in both places
> instead of discovering the router needs one later. Plan-level buddy-check (two reviewers, PHASE
> = spec) converged after one round (1 Important + 3 Minor folded into the plan before execution).
> Task 1 (the mask-parity fix) was itself pre-authorized for a per-task buddy check (two reviewers,
> PHASE = code); one Important/Minor-disputed finding (a doc comment briefly overclaiming the
> region hook as already wired) was fixed and reverified. Task 2 (the region stub) passed a normal
> two-reviewer pass clean. A final whole-checkpoint review (opus spec-lens + code-lens over the
> full M3 diff) found zero further issues. Plan:
> `docs/superpowers/plans/2026-07-01-m3-vision-gated-pathfinder.md`.
> Spec: `docs/superpowers/specs/2026-07-01-m3-vision-gated-pathfinder-design.md`.
>
> **M10e status: e-1 through e-6 DONE; the M10e-5 server-authoritative-movement redirect (M1 + M2 +
> M3) is fully DONE.**
>
> **M10g DONE** (merged --no-ff to LOCAL main `ba1dfcf`, NOT pushed — push gate = full M10) —
> weighted/impassable/hazard-arrest regions, **grid engine only**. Spec:
> `superpowers/specs/2026-07-01-m10g-regions-design.md`. Shipped: three behaviors (`terrain`
> per-cell cost multiplier / `impassable` / `arrest` stop-on-enter); per-region secrecy via
> envelope permission tiers (secret regions absent from a player's router/budget field, sprung by
> `move_exec`; `permissions.default="none"` drops a secret region's whole Create op at egress, not
> just a `/system` field-null — a Critical caught + fixed in the final whole-branch review);
> vector-shape authoring (rect/circle/polygon, rasterized to cells); precedence+MAX overlap
> compose; honest arrest preview (`PathResult.arrested` truncation); GM region-authoring tool +
> `RegionView` render layer. Lit up the planted inert `region_arrests()` hooks in
> `scene/move_exec.rs` + `scene/pathfinding.rs` as a matched pair. No new crate (cargo-bloat
> untouched). SDD-executed (13 tasks, per-task two-reviewer gate incl. 4 buddy-checked tasks +
> whole-branch review). **Three items EXPLICITLY DEFERRED from M10g and homed below so they are not
> lost:** (a) Polyanya/navmesh cost-layers → **M10f-4**; (b) per-actor/faction movement exemptions →
> **Phase 2 vision/lighting/movement completion**; (c) mechanical/trigger effects on arrest →
> **Phase 2 trigger regions**.
>
> **M10f (continuous/navmesh movement) SPEC'D (design approved 2026-07-02)** — the M10e-5
> server-authoritative movement redirect (M1/M2/M3) made M10f bigger than the original "adopt
> `vleue/polyanya`" line item: the whole movement stack (router + gated execution + streamed
> vision) is grid-cell-based end to end, so continuous movement needs its own decomposition, not a
> second router bolted on. Spec: `superpowers/specs/2026-07-02-m10f-continuous-navmesh-movement-
> design.md`. Locked: full continuous stack, decomposed not descoped; **cell-sampled gate** — the
> polyanya router's any-angle polyline is arc-length-sampled and gated against the SAME
> `visible_cells` cell mask grid movement uses (§13 never-fork preserved; continuous changes
> geometry/distance-metric only, never the secrecy decision); **one unified sampled executor** —
> `move_exec`'s king-step walk generalizes to an arc-length-sampled polyline walk so grid A* and
> polyanya share one gate/region/arrest/commit path (grid = the ≤1-cell-apart special case);
> **explicit scene bounds primitive** (a navmesh needs a bounded triangulation region; grid A* never
> did). Decomposed **M10f-0 → M10f-4**; region cost-layers (deferred from M10g above) land in
> **M10f-4**.
>
> **M10f-0 DONE** (branch `m10f-0-scene-bounds`, commits `7afd610..2401783`, all green, NOT
> merged/pushed — merge gate = full M10f) — scene bounds primitive: `scene.system.bounds
> {width,height}` (grid units), mirrored client (`scene-docs.ts` `SceneDimensions`/
> `DEFAULT_SCENE_BOUNDS`, deep-frozen) + server (`scene/mod.rs` `ResolvedScene.bounds`/
> `DEFAULT_SCENE_BOUNDS_UNITS`), both fail-closed to a `100×100` grid-unit default (non-finite or
> ≤0-on-either-axis never produces a degenerate rectangle); per-scene only, no world-settings layer,
> deliberately NOT content-derived (rejected at design time: edge-drag re-mesh churn, ill-defined
> for open scenes); GM width/height authoring control in `module-game-settings`. SDD-executed (3
> tasks, per-task two-reviewer gate — Task 1 had one Important fixed [`DEFAULT_SCENE_BOUNDS` not
> deep-frozen, a shared-mutation risk via the fail-closed fallback's shared reference]; Task 3 found
> + fixed an incidental Task-1-left-behind `@shadowcat/core` barrel-export gap, precedent-matched to
> the M10g region export gap). Whole-branch review: Ready to merge, zero Critical/Important.
> **Unblocks** (but does not itself implement) the M10e-2 edge-projected-environment-light
> deviation — that implementation stays homed to M12 (logged `docs/TODO.md`). Reviewed
> skill-update gate: `shadowcat-codebase-scene-rendering` updated + confirmed ACCURATE.
>
> **M10f-1 DONE** (branch `m10f-1-movement-model-dispatch`, commits `008e8e2..080de7f`, all green,
> merged --no-ff to LOCAL main — merge gate = full M10f) — `movementModel` scene axis
> (`grid-stepped` default, `continuous` opt-in; server `MovementModel`/`parse_movement_model` +
> client `MovementModel` type, world-default + per-scene override, resolved exactly like
> `movement_restriction`, fail-closed to `grid-stepped`) + a headless `polyanya`-navmesh router
> dispatched alongside the existing grid A*. `scene/navmesh.rs` (new): `build_navmesh` (bounds +
> `blocksMove`-wall footprint-inflated obstacles, `geo::Buffer` + `polyanya::Triangulation`),
> `navmesh_find` (any-angle multi-leg routing, Euclidean cost), `clip_to_visible_mask` (the
> security-critical fog-safe + wall-safe route-preview post-filter — arc-length-samples the route
> and truncates at the first sample that leaves the requester's `visible_cells` mask OR whose chord
> crosses a wall; reuses the SAME mask `SceneEcs::pathfind` builds once and shares across both
> engines — never forked, generalizing the M10e-6 §13 invariant to a second routing engine).
> Per-`(scene, quantized footprint radius)` memoized navmesh cache on `SceneEcs`
> (`std::sync::Mutex`+`Arc`, invalidated wholesale on any `wall`/`scene` document mutation via
> `apply_op`). New deps: `polyanya` (headless CDT-backed any-angle navmesh, default-features off),
> `geo` (pinned to polyanya's own copy), `glam` (direct dep for `Vec2`, required even though
> polyanya pulls it transitively) — binary-size delta ~0.94 MiB, well under the 60 MiB CI budget.
> **Preview-only checkpoint, by design:** continuous scenes get the router + an honest fog-safe
> route preview + Euclidean budget; committing (executing) a continuous-scene move is explicitly
> disabled client-side (`commitRoute` gate in `controller.svelte.ts`, checked via
> `resolveSceneSettings`) — no grid-snap fallback — since continuous move *execution* is a later
> checkpoint (M10f-2/3). SDD-executed (10 tasks; Tasks 4/5/6/7 buddy-checked per the plan's
> pre-authorization, Tasks 1-3/8-10 the standard two-reviewer gate). **Buddy-checking found and
> fixed 6 distinct Critical/Important defects**, all independently re-confirmed resolved: three
> separate `f64→f32` cast-saturation panics inside `spade`'s triangulation (unbounded `bounds`/
> `cell`, then unbounded `footprint_scene`, each requiring its own fix round after the prior round's
> guard proved incomplete) closed by a `MAX_NAVMESH_COORD` bound now covering every coordinate
> surface that reaches a narrowing cast in the module; a **silent fail-open** where a wall obstacle
> could vanish from the mesh entirely under ordinary-looking inputs (a `geo`/`i_overlay` fixed-point
> quantization collapse, verified by both empirical worktree reproduction and independent
> source-level derivation — arguably more severe than a panic, since it is invisible) closed by
> treating an empty Minkowski-buffer result for a well-formed wall as a hard build failure; a
> `NaN`/negative-radius navmesh-cache-key collision letting a degenerate footprint radius silently
> return an already-cached valid mesh, closed by validating before the cache lookup rather than
> after; and a grid/continuous engine-parity gap where routing to your own current position
> succeeded on grid-stepped scenes but failed as `Unreachable` on continuous ones, closed via a
> `raw_was_trivial` flag distinguishing a genuine zero-cost success from a real mask/wall rejection.
> The plan document itself was buddy-checked BEFORE execution began (PHASE=spec), catching an
> exact-f64-bits cache-key-quantization design flaw and a missing DoS-bound requirement before any
> code was written. Final whole-branch review (opus, two-reviewer): clean integration, no
> cross-task regressions, all per-task fixes intact in the assembled tree; one Minor (a
> system-level test gap on the non-GM continuous secrecy path — closed with a dedicated
> `pathfind`-level test proving the real `visible_cells` mask reaches `clip_to_visible_mask` through
> the full dispatch chain, not just a hand-built mask in a unit test). Reviewed skill-update gate:
> `shadowcat-codebase-scene-rendering` updated + confirmed ACCURATE (one round of Minor
> corrections applied: an invariant promoted from prose to a top-level Hard Invariants entry, an
> unverifiable buddy-check round-count claim softened).
>
> **M10f-2 DONE** (branch `m10f-2-unified-movement-executor`, commits `a343fd0..53335eb`, all
> green, merged --no-ff to LOCAL main — merge gate = full M10f) — unifies the movement executor:
> `move_exec` refactored from a king-step-per-authored-cell walk onto a new pure `gate_walk`
> primitive that subdivides ANY polyline (grid A* cell-centers or any-angle continuous vertices)
> into a dense walk where consecutive samples are ≤1 cell apart, preserving already-≤1-cell input
> EXACTLY — identity on grid input, which is what makes grid-parity a property of the code shape
> rather than something proven only by testing. The per-step gate (wall → vision-mask → region)
> now runs over this dense walk instead of the raw authored path; region/cost checks are keyed on
> CELL-ENTRY TRANSITIONS (not per dense sample) to match the pre-refactor per-step accrual exactly
> on grid input. The old king-step adjacency guard (reject any >1-cell authored jump outright) is
> REMOVED — a >1-cell jump is now subdivided and gated per cell instead, exactly as if the client
> had sent the explicit intermediate waypoints (no new capability). The DoS bound moves from an
> authored-vertex-count cap (`MAX_MOVE_PATH=256`) to a gate-walk-sample-count cap
> (`MAX_GATE_WALK_SAMPLES=4096`) plus a coordinate-magnitude bound (`MAX_GATE_WALK_COORD=1e9`).
> `Room::execute_move` required **zero code changes** throughout — the caller seam is unchanged,
> proven by its full existing test suite staying green across all 6 tasks. SDD-executed (6 tasks;
> Tasks 1, 3, 4, 5 buddy-checked per the plan's pre-authorization, Task 2/6 the standard
> two-reviewer gate, plus a mandatory whole-branch buddy-check before the final task).
> **Buddy-checking found and fixed 2 distinct real defects in Task 1's `gate_walk` primitive**,
> both independently re-confirmed resolved: a zero-tolerance floating-point comparison
> (`cheby <= cell`) that spuriously subdivided a true 1-cell grid step for non-round GM-configured
> cell sizes (empirically reproduced: 1087/2000 spurious subdivisions at `cell=33.33`), closed by a
> magnitude-scaled tolerance matching `supercover_cells`'s existing 64-ULP convention; and a
> **second-order defect the first fix itself introduced** — the magnitude-scaled tolerance grew
> unbounded with coordinate magnitude and, at extreme-but-reachable magnitudes (~3.5e13+, within
> `navmesh.rs`'s own `MAX_NAVMESH_COORD=1e15` legitimate-input band), could silently misclassify a
> genuinely-multi-cell segment as a single identity step — a silent gate-skip — closed by the new
> `MAX_GATE_WALK_COORD=1e9` bound (~35,000x margin below the crossover). Task 3's refactor buddy
> check found zero functional defects, only 3 doc-only Minors (one requiring a real severity
> debate that converged after both reviewers independently re-verified against ground truth). The
> mandatory whole-branch buddy-check (opus, two-reviewer) independently confirmed all 4 directed
> risks safe with real margin (grid-input identity, cell-entry-dedup re-entry correctness, the
> `render_path` reconstruction's edge cases, and the `TooLong` redefinition's DoS coverage) —
> CONVERGED PASS, zero Critical/Important. **A genuinely valuable process catch during Task 6:**
> the differential parity test's hand-derived literal for one scenario (a diagonal 3-step king
> path) was wrong — root-caused to a real, pre-existing `supercover_cells` corner-drift (a
> diagonal leg whose both endpoints sit exactly on 4-way grid intersections drives the
> Amanatides-Woo corner-crossing branch to fire repeatedly and drift away from the target cell
> before failing closed) — the implementer correctly halted rather than silently "fixing" the
> literal, and the dispatcher + both Task 6 reviewers independently re-derived the same correction
> from the actual geometry. Fail-closed, non-security, logged to `docs/TODO.md`. Reviewed
> skill-update gate: `shadowcat-codebase-scene-rendering` updated + confirmed ACCURATE.
>
> **M10f-3 DONE** (branch `m10f-3-continuous-execution-snap-toggle`, commits `eb2cea9..f6571ca`,
> all green, merge gate = full M10f) — a client-weighted checkpoint: unlocks server-authoritative
> continuous (any-angle) movement execution and adds an independent scene-level `snapToGrid`
> authoring axis. **The server needed ZERO production code changes** — `Room::execute_move`,
> `move_exec::execute_move`/`gate_walk`, `move_stream::sample_path`, and the M2 `clip_move_stream`
> egress clip have had NO `movementModel` branch anywhere since M10f-2, so the entire move-
> execution/streaming/secrecy-clip path already gated, executed, and clipped any polyline (grid or
> any-angle) correctly. The only thing blocking continuous execution was a client-side refusal:
> `controller.svelte.ts`'s `commitRoute` removed its M10f-1 preview-only early-return, so committing
> a route now proceeds identically for grid-stepped and continuous scenes. M10f-3's own server work
> is therefore TEST-ONLY (Tasks 7-9): new `Room::execute_move`/`sample_path`/`clip_move_stream`
> coverage empirically proving the already-engine-agnostic path handles any-angle geometry. New
> **`snapToGrid`** scene axis (`scene-docs.ts`, opaque `system`-body JSON, no ts-rs type): a
> `resolveSceneSettings`-derived default keyed off `movementModel` (`false` for continuous, `true`
> otherwise, unless explicitly overridden in either direction — nullish-coalescing, never a truthy
> check), enforced at a SINGLE chokepoint (`RenderEngine.snap`, gated by a new
> `SceneToolHost.setSnapEnabled` seam forwarded through `SceneInteractionBridge` and pushed
> unconditionally from `Stage.svelte`'s existing per-pass scene-settings effect) that every scene
> tool inherits automatically via `ctx.scene.snap`; authored via a new GM-only tool-rail toggle
> button. SDD-executed (9 tasks; Tasks 6-9 buddy-checked per the plan's pre-authorization, Tasks
> 1-5 the standard two-reviewer gate, plus a mandatory whole-branch buddy-check before merge).
> **Task 5 review caught a real Critical**: the tool-rail toggle hardcoded `old: null` in its
> dispatched scene-doc update, so the server's field-level optimistic-concurrency check
> (`Repository::apply_intent`) rejected every click after the first (the actual stored value is no
> longer `null`/absent once written once) — the toggle silently stopped working after one use per
> scene per session; fixed by reading the RAW stored value for `old` (mirroring the existing
> `sendMoves` convention), with a new regression test confirmed genuinely discriminating (would
> have failed pre-fix) by the reviewer. The SAME pre-existing bug shape was found (not introduced)
> in `GameSettingsPanel`/`FactionsPanel`/`ConditionsPanel` — logged to `docs/TODO.md` rather than
> fixed (out of scope), per this project's bug/TODO segregation discipline. **Buddy-checking on
> Tasks 6-9 was genuinely valuable, not rubber-stamped:** Task 6's reviewers independently verified
> the central safety claim (no `movementModel` branch anywhere in the execution chain) directly
> against server source rather than trusting the diff, and both independently found a distinct
> stale-doc Minor the other missed (both conceded + fixed). Task 7's brief itself assumed the
> wrong server behavior (`Err(Forbidden)` for a cell-gate rejection) — the implementer discovered
> `execute_move` actually TRUNCATES (never errors) for a wall/mask/region violation, matching the
> same class of "brief literal wrong, halt and verify against real geometry" precedent as M10f-2
> Task 6; both reviewers independently re-verified this from source and confirmed the rewritten
> test is a faithful, non-weakened proof, then jointly caught and fixed a stale "Forbidden" wording
> bug surviving in TWO approved design docs. Task 8's buddy check caught a genuinely load-bearing
> gap: the test asserted only angle-independent properties (monotonic time, trivial boundary
> positions) and never checked any INTERIOR sample's position, so it would not have caught a real
> diagonal-interpolation bug; one reviewer's specific supporting claim (a differing binary-search
> branch pattern between the new and sibling tests) was directly disproven by the other via
> simulation before the fix was scoped, then BOTH reviewers independently re-derived the fix's hand-
> computed interior-sample arithmetic from scratch and matched it bit-for-bit. Task 9's buddy check
> caught the sibling-parity gap for `duration_ms` (disagreed on severity, agreed on the fix
> regardless) plus converged 4 other Minors to no-action after real geometric re-derivation. The
> mandatory whole-branch buddy-check (opus, two-reviewer) traced all 3 directed risks against
> actual current source and confirmed clean — CONVERGED PASS, zero Critical/Important code defects
> — with one Agreed Minor (non-GM invisibility of the snap toggle untested) whose OWN proposed
> naive fix was caught as vacuous mid-debate (the existing non-GM test's fixture has no active
> scene, so a different gate already hides the button regardless of the `isGm` check) and correctly
> replaced with a genuinely isolating test before the fix was dispatched. Deviation from parent
> spec §9 recorded: §9 tied "no snap" to `movementModel`; this checkpoint supersedes that with an
> independent scene-level toggle (the derived default preserves §9's original intent). Reviewed
> skill-update gate: `shadowcat-codebase-scene-rendering` updated (the `snapToGrid` axis + the
> `RenderEngine.snap`/`setSnapEnabled` chokepoint + the raw-stored-value `old` convention) +
> confirmed ACCURATE.
>
> **M10f-4 DONE** (branch `m10f-4-regions-on-navmesh`, commits `c20cffa..47419c3`, all green —
> merge gate = full M10f) — **final checkpoint of the M10f milestone; M10f is now COMPLETE.** Wires
> the M10g region behaviors (terrain/impassable/arrest) into the continuous engine, correcting the
> original two-cost-backend assumption (M10-tokens §10.2/§10.3/§10.5, parent M10f spec §7): the
> M10g per-requester `region_field(scene, viewer)` cell field is the **single weighting authority
> for both engines** — polyanya 0.16.1 cannot bias a route by graded terrain cost
> (crate-source-verified, not README-derived; the only cost-affecting knob, `detailed-layers`'
> `Layer.scale`, is off in this build and semantically wrong as a per-unit multiplier anyway), so
> the "terrain → polyanya cost-layer (Split-Mesh)" plan is struck as infeasible, not deferred.
> `SceneEcs::pathfind`'s `Continuous` branch (`mod.rs`) now computes the per-requester
> `region_field` once and dispatches on the new `RegionField::has_terrain_or_impassable()`
> predicate (`regions.rs`): **terrain/impassable present** → the existing `pathfinding::find`
> forced to `DiagonalRule::Euclidean` (continuous base metric; only cell topology + the terrain
> multiplier come from the grid), cost converted from CELLS to SCENE UNITS (`× cell`, matching the
> polyanya path's unit contract), then **cost-guarded LOS smoothing** (`navmesh::los_smooth`, new)
> restores any-angle geometry — a span straightens only when every entered cell is in-mask, not
> impassable/arrest/weighted-terrain, and crossed by no wall, with the single grid step always kept
> unconditionally so progress is guaranteed; **otherwise** the unchanged pure-polyanya route (M10f-1)
> now also passed through `navmesh::truncate_at_arrest` (new), an arrest post-filter using
> cell-ENTRY-TRANSITION detection (not raw per-sample checking — the start cell is never a trigger),
> mirroring `find`'s own arrest truncation for the walls-only path. The `GridStepped` branch is
> completely untouched (verified byte-for-byte by a dedicated regression test throughout).
> `move_exec::execute_move` required **zero production changes** — proven, not merely asserted, by
> Task 5's tests: it has cell-sampled the region field for any polyline (grid or any-angle) since
> M10f-2/3, so the weighted+smoothed continuous route feeds the same executor unchanged. Secrecy is
> fully inherited (zero new machinery, per spec §8): the per-requester `region_field` feeds BOTH the
> dispatch predicate and the weighted search/arrest-filter, so a secret region never influences a
> non-GM's route/cost; the authoritative field (`move_exec` always reads `viewer: None`) springs it
> at execution regardless of what the requester's preview showed. `MoveStream.cost` stays
> trusted-only (`Some` for mover/GM, `None` for a clipped observer) — verified engine-agnostic by
> Task 5, extending the M10g whole-move-scalar invariant to the continuous path. SDD-executed (6
> tasks: 5 code/test tasks + this docs task; no buddy-checked tasks pre-authorized at the plan level
> for this checkpoint — see the separate mandatory whole-branch buddy-check before merge). **A
> load-bearing dispatch-predicate invariant surfaced during Task 4's review:** the dispatch
> predicate `has_terrain_or_impassable()` MUST be evaluated against the PER-REQUESTER field
> (`region_field(scene, Some(user))` for a non-GM), never the authoritative field — this is the
> single mechanism preventing a secret terrain/impassable region from indirectly leaking its
> existence via route-shape or cost even when its own geometry is never disclosed; a future refactor
> that fed the authoritative field into the dispatch predicate while still correctly routing off the
> per-requester field would silently reopen this leak. Reviewed skill-update gate:
> `shadowcat-codebase-scene-rendering` updated + confirmed ACCURATE. **Push gate: full M10** — M10f
> is complete but M10 (which also includes the parallel M10g-adjacent work) is not yet pushed to
> origin; push happens at the M10 milestone boundary, not per-checkpoint.
>
> **M10f (continuous/navmesh movement) COMPLETE** — all five checkpoints (M10f-0 scene bounds,
> M10f-1 movement-model dispatch + polyanya router, M10f-2 unified executor, M10f-3 continuous
> execution + snap toggle, M10f-4 regions on the navmesh) shipped. Both routing engines (grid A*,
> continuous/polyanya) now share one region-weighting authority, one gated executor, and one
> streamed-vision secrecy clip.
>
> **M10h DONE** (branch `m10h-faces-animated`, commits `5214892..a577570`, all green) — **faces +
> animated token visuals; purely client-side, no server/ts-rs change** (the `system`-body visual
> data is opaque client-owned JSON, same convention as `movementModel`/`bounds`/`snapToGrid`).
> Spec: `docs/superpowers/specs/2026-07-03-m10h-faces-animated-design.md`. Replaces the old flat
> `ActorVisual` with a discriminated union: `RenderVisual = {kind:"image", asset} | {kind:"animated",
> source: AnimatedSource, fps, loop}` (the two kinds the render layer ever draws); `FaceVisual =
> RenderVisual` (a face is never itself nested `{kind:"faces"}` — no faces-of-faces); `TokenVisual
> = RenderVisual | {kind:"faces", faces, default, faceMap?}` (`default` is required, no `?`); new
> per-token `token.system.face?`
> active-face selector (token-local, not part of `overrides` — selects INTO the actor's faces map
> rather than overriding actor data). New render-boundary resolver `resolveTokenVisual(token,
> store, eff?)` (sibling to `resolveTokenActor`/`resolveTokenBox`/`resolveConditions`) applies
> precedence manual `token.system.face` > first `faceMap` match against the token's raw
> `conditions[]` (array order) > `default` > first key, and fails closed (`null`) on an empty
> `faces` map, a malformed `AnimatedSource`, or a resolved kind outside `image`/`animated`. New
> pure `computeAnimatedFrame` (`token-animation.ts`, mirrors the `fog-blend.ts` extraction
> precedent — `pixi-backend.ts` has no jsdom GL context) drives tick-based `AnimatedSprite`
> playback via the new `DisplayBackend.tickTokenAnimations(dtMs)` seam, called from
> `TokenView.tick` alongside the existing tween ticker. `TokenNodeSpec.visual` becomes a
> discriminated union (`{kind:"image",url} | {kind:"animated",source: ResolvedAnimatedSource,fps,
> loop}`), asset ids resolved to URLs by `TokenView.toSpec`'s `resolveSource` via `AssetResolver`.
> `PixiBackend` migrated from a bare `Sprite`-per-token + three separately-tracked sibling Maps to
> a `Map<string,TokenNode>` Container structure (`container` outer/non-rotating, holds upright
> badges directly; `visualContainer` inner, rotates the art + border together). **Real bug found +
> fixed in review:** the async texture/frame-load completion guards originally checked only
> `sourceKey` string equality, unsafe once `replaceVisualChild` could recreate a token's visual
> object multiple times in rapid succession (an A→B→A visual-cycling scenario could let a stale
> promise write into an already-`.destroy()`'d Pixi object) — fixed by also requiring object
> identity (`node.visual === sprite`), now a load-bearing invariant for this async-completion
> pattern generally. Authoring UI in `ActorsPanel.svelte`: a visual-kind editor (image/faces/
> animated) in the actor-creation form with full per-face-row/name-uniqueness/`defaultFace`
> validation, plus a separate per-token face-swap palette (reading raw `token.system.face` for
> `old`, mirroring the M10f-3 `snapToGrid` raw-`old` convention). A Playwright e2e test proves an
> animated (frame-list) actor authors and places on canvas without error; `stage.spec.ts` (the
> relevant token/scene-tools e2e coverage) totals 8 tests (7 pre-existing + 1 new), all passing,
> confirming the Container migration didn't regress prior scene-tools/token behavior (the full
> e2e directory across all spec files totals 10). SDD-executed (9 code/test tasks + this docs
> task). Reviewed skill-update gate: `shadowcat-codebase-actors-tokens` +
> `shadowcat-codebase-scene-rendering` updated; `shadowcat-spec-reviewer` found and this fix
> corrected 3 real drifts (`RenderVisual`'s image variant missing its `asset` field name,
> `TokenVisual`'s `faces.default` wrongly marked optional, and this entry's premature "confirmed
> ACCURATE" claim predating the actual review) before confirming ACCURATE. **M10h merged --no-ff
> and PUSHED to origin/main (`50df79f`, 2026-07-04, by explicit user override of the standing
> full-M10 push gate).**
>
> **M10 CONCLUDED here (2026-07-04, user decision): the two remaining visual-polish checkpoints —
> M10i (`generated` parametric token visual) and M10j (`fx` + emotes) — are DEFERRED to Phase 2
> (token enrichment), not built. Their seams already exist and need no preparatory code:** the
> `RenderVisual` discriminated union is additive and fails closed on unknown kinds, so a future
> `{kind:"generated"}` is forward-compatible (`resolveTokenVisual` renders nothing rather than
> crashing on an old client); every token has been a Pixi `Container` (`node.visualContainer`)
> since M10h, so a per-token `.filters` attach point is one additive method beyond the existing
> per-layer `addLayerFilter` (`fx`); and the `broadcast_aux`/`ScenePing` aux-frame pattern is the
> direct template for a new transient `emote` frame (emotes). **Note on `generated`'s intended
> meaning** (user-clarified at deferral time): a *compositor that frames existing actor art into a
> token* — decorative border + shape-crop + background, distinct from the dynamic faction ring —
> NOT the parent spec's literal "shapes/initials for artless actors" reading.
- Actor-linked tokens; shapes; instanced / unique modes; A* pathfinding with waypoints; status conditions; factions.
- Realizes the token-visual architecture seeded in M8 on top of M8d's sprite/tween/ticker
  foundation: **multi-face + animated visuals SHIPPED** (M10h); **procedurally-generated visuals,
  per-token fx, and emotes DEFERRED to Phase 2** (seams in place).

### M11 · Dice + chat
Two subsystems (dice → chat; chat's roll integration depends on dice). Specs:
[`superpowers/specs/2026-07-03-m11-dice-engine-design.md`](superpowers/specs/2026-07-03-m11-dice-engine-design.md)
+ [`superpowers/specs/2026-07-03-m11-chat-system-design.md`](superpowers/specs/2026-07-03-m11-chat-system-design.md).
Decomposed **M11a–d**:
> **M11a DONE** (branch `m11-dice-and-chat`, 11 SDD tasks + a codebase-skill gate, all green) —
> the pure-Rust dice engine at `src/server/src/dice/`: hand-rolled seeded-noise RNG (SplitMix64,
> no `rand` dependency — user preference for determinism-by-construction), `RollSpec`/`Expr` AST,
> `roll`/`evaluate`/`recalculate`, Sum + SuccessCount modes, and a recursive-descent notation
> parser (`4d6kh3+2` style). **Purely additive, zero server/ws/data coupling** — no wire frames,
> no ts-rs bindings (M11a/b stay pure; wire integration is M11d). **3 pre-approved buddy-checks
> (Tasks 4/6/10) each found and fixed real Critical/Important bugs** — this milestone is further
> evidence that pre-authorizing buddy-check on dense pipeline/algorithmic-core code pays for
> itself: Task 4 (group pipeline) caught an outer-loop explosion double-trigger causing unbounded
> dice growth (empirically reproduced at 41GB memory) + a Penetrate retrigger-on-decremented-value
> bug that silently truncated Penetrate chains to length 1 + a `kept`-flag bypass letting a
> Drop-then-Reroll sequence mutate an already-dropped die; Task 6 (`group_index` Sum-mode fold)
> caught two test-coverage gaps on the plan's own flagged "group-boundary reconstruction is the
> correctness core" risk (a commutative-op multi-group test that couldn't detect a mis-assignment;
> missing coverage for `group_index` propagating through exploded/penetrated children); Task 10
> (`recalculate`, "the highest-consequence correctness path in M11a") confirmed the
> untouched-sibling-derived-tail-changes-across-recalc behavior was correctly designed per the
> approved plan but had ZERO test coverage, closed with 3 pinning tests. Two more Important bugs
> caught in single-reviewer passes: Task 8's lexer didn't lex uppercase `D` as the dice operator
> (case-inconsistent with the rest of the lexer) and had an unenforced (accidentally-safe)
> ASCII-only assumption; Task 9's parser silently overwrote a duplicate `cs`/`cf` success rule
> instead of erroring. **Mandatory whole-branch final review: Approved, zero Critical/Important**
> — confirmed all six fix rounds compose correctly through the single shared `resolve_group` entry
> point (used by both `roll` and `recalculate`'s `rederive`), full `dice::` suite green (51 tests),
> pure-library/determinism invariants held, and the plan's M11b deferrals (expertise DP, crit
> events, Tiered mode, labeled/custom-face dice, `direction`) are genuinely absent, not
> half-built. New `shadowcat-codebase-dice` skill created + confirmed ACCURATE by
> `shadowcat-spec-reviewer`. **M11a — Dice engine core:** server-authoritative Rust evaluator over a declarative struct-canonical `RollSpec`; seeded-noise RNG (deterministic, no `rand`); rpg-dice-roller-superset notation; Sum + SuccessCount modes; `roll`/`evaluate`/`recalculate`; pure library (tests only). Plan: [`superpowers/plans/2026-07-03-m11a-dice-engine-core.md`](superpowers/plans/2026-07-03-m11a-dice-engine-core.md).
- **M11b — System rules**, decomposed into checkpoints:
  > **M11b-1 DONE** (branch `m11-dice-and-chat`) — globals + shared classification + crit events:
  > `direction: Direction` (`HighWins`/`LowWins`) global flip on `RollSpec`; `Mode` made
  > data-carrying (`Total(TotalConfig) | SuccessCount(SuccessConfig)`, unifying Sum and Tiered into
  > one mode with an optional custom ladder); the shared `eval::classify::{classify,
  > oriented_margin}` layer used by both modes (`oriented_margin` Total-only, SuccessCount's margin
  > deliberately not direction-flipped); crit events (`eval::crit::score_die`, net-success
  > clamp-at-zero unless `allow_negative`, counters as a separate output from successes,
  > overlapping crit-success/crit-fail intentionally both-fireable); unified `t<N>` notation +
  > ambient `ParseContext` (mode/direction-derived comparator). Plan:
  > [`superpowers/plans/2026-07-04-m11b-1-globals-classification-crit.md`](superpowers/plans/2026-07-04-m11b-1-globals-classification-crit.md).
  > **M11b-2 DONE** (branch `m11b-2-expertise-dp`) — provably-optimal expertise-point DP allocator:
  > `SuccessConfig.expertise: u32` / `DieRecord.expertise: i32`; `eval::expertise::{adjust,
  > die_values, run_dp, allocate}` — a bounded lexicographic knapsack DP (`O(N·E²)`) maximizing the
  > CLAMPED (visible) net successes with a counter-max fallback in the all-failed region, tie-broken
  > deterministically (reversed-lexicographically-smallest allocation, refined during the mandatory
  > buddy-check from the original plan's forced-full-spend characterization), wired into
  > `evaluate_success` as a value-mutating pre-pass before the sealed M11b-1 counting logic;
  > `e<N>` notation (roll-level, mode-lenient — silently discarded under Total, never a parse
  > error). Mandatory brute-force differential oracle (4000-case corpus) proved DP==oracle on both
  > objective value AND exact per-die allocation — no assertion relaxed. Design:
  > [`superpowers/specs/2026-07-04-m11b-2-expertise-dp-design.md`](superpowers/specs/2026-07-04-m11b-2-expertise-dp-design.md).
  > Plan: [`superpowers/plans/2026-07-04-m11b-2-expertise-dp.md`](superpowers/plans/2026-07-04-m11b-2-expertise-dp.md).
  > **M11b-3 DONE** (branch `m11b-3-labeled-custom-face-dice`, 13 SDD tasks + a codebase-skill gate,
  > all green) — labeled dice (`DiceGroup.label`/`DieRecord.label`, propagated through
  > `resolve_group`/exploded children; `RollOutcome::by_label`/`compare_labels`; `[label]` notation,
  > case-preserving, ASCII-printable-except-`]` charset) + custom-face (symbolic) dice
  > (`DieKind::Faces{faces: Vec<Face>}`, `Face{value: Option<i32>, symbols: Vec<Symbol>}`, RNG draws
  > a face INDEX; `DieKind::is_ordered()` gates `resolve_group`'s entire modifier loop fail-closed
  > for any group with an unordered face; `Compound`/`Penetrate` explode restricted to `Numeric`
  > only, an ordered `Faces` die falls through to Standard-style push) + `SuccessRule`/`CritTrigger`
  > promoted to enums (`Numeric`/`HasSymbol` success rule; `AtLeast`/`HasSymbol` crit trigger,
  > `HasSymbol` direction-insensitive) + unconditional `symbol_counts` + expertise restricted to
  > `Numeric` dice, folding any excluded `Faces` die's fixed contribution into the two-pass
  > clamp-decision threshold. **Mandatory buddy-check (Task 9, reopening the sealed M11b-1
  > crit-scoring path for `CritTrigger`)** converged clean; two more real bugs surfaced and were
  > fixed within their own tasks: Task 6 (Explode retrigger on an ordered `Faces` die must test the
  > die's derived value, not the raw drawn index — the raw-index check would silently misfire
  > whenever face value doesn't track index order), found in the implementer's own self-review and
  > confirmed fixed by both task-scoped reviewers; and Task 11 (expertise's all-failed-region branch
  > check omitted a fixed contribution from an excluded kept `Faces` die, answering a different
  > question than `evaluate_success` would actually score), found by the single-reviewer spec pass.
  > Design:
  > [`superpowers/specs/2026-07-07-m11b-3-labeled-custom-face-dice-design.md`](superpowers/specs/2026-07-07-m11b-3-labeled-custom-face-dice-design.md).
  > Plan: [`superpowers/plans/2026-07-07-m11b-3-labeled-custom-face-dice.md`](superpowers/plans/2026-07-07-m11b-3-labeled-custom-face-dice.md).
  > **M11b is now fully DONE** (M11a + M11b-1 + M11b-2 + M11b-3). `shadowcat-codebase-dice` skill
  > updated for M11b-3, reviewed by `shadowcat-spec-reviewer` per the reviewed skill-update gate.
  Spec: [`superpowers/specs/2026-07-04-m11b-system-rules-design.md`](superpowers/specs/2026-07-04-m11b-system-rules-design.md).
- **M11c — Chat core (headless)**, decomposed into checkpoints:
  > **M11c-1 DONE** (branch `m11c-1-message-model`) — message model + server-authoritative
  > ingest + delivery: a chat message is an ordinary sequenced `Document` (`doc_type: "message"`)
  > riding the existing Event/redaction/search path with zero message-specific plumbing in any of
  > those subsystems. `MessageSystem{channel, user_owner, actor_owner, kind, content}` body;
  > `Segment` content model (`Text`-only in c-1, tagged enum, opaque JSON, deliberately NOT
  > ts-rs-exported — the client mirrors it independently in Zod later, M11d); `ActorOwnerRef`
  > (`Actor`/`TokenInstance`) is the SOLE chat type with a ts-rs binding, carried on the
  > `SendMessage` wire frame. `chat::handle_send_message` → `chat::build_message_doc` →
  > `Room::publish` is the SOLE authoring path for a stored message doc — the client never
  > constructs one, only sends `ClientMsg::SendMessage`; validates empty/`MAX_MESSAGE_CHARS=4096`/
  > a per-user-per-minute flood budget (`PingRateLimiter`) before publishing. Authz is a COUPLED
  > two-chokepoint seam: (1) a Player-baseline `core:create` exemption in `apply_intent`
  > (`sqlite.rs`) lets a Player create a message doc only when self-owned, made sound ONLY because
  > (2) `chat::ops_target_message` rejects any client-authored `message` Create/Delete at BOTH the
  > WS `Intent` and HTTP `write_ops` ingress boundaries before that exemption is ever reached —
  > weakening either chokepoint alone reopens forgery. A third, independent chokepoint
  > blanket-rejects every client `Update` against a stored `message` doc in `apply_intent`
  > (keyed on the STORED doc_type, since `Update` carries none), even the owning Player's own
  > message — a deliberate placeholder pending c-3's validated edit path. `shadowcat-codebase-chat`
  > skill created + reviewed by `shadowcat-spec-reviewer` per the reviewed skill-update gate.
  > **M11c-2 (restricted-audience messaging — whisper + GM-only channel) is next**, design widened
  > from a plain whisper allowlist during brainstorming: adds `PermissionSet.gm_role:
  > Option<DocRole>` (default `None`, zero behavior change elsewhere) so `resolve_access`'s GM
  > short-circuit becomes conditional per-document, reusing the M10g `default: DocRole::None`
  > whole-doc-suppression precedent for both a sender-picked whisper recipient list (GM excluded
  > unless named) and a dynamically-resolved GM-only channel (any current GM sees it, no frozen
  > roster) — zero changes needed to any of the four egress call sites.
  > Design: [`superpowers/specs/2026-07-08-m11c-2-whisper-allowlist-design.md`](superpowers/specs/2026-07-08-m11c-2-whisper-allowlist-design.md).
  Design: [`superpowers/specs/2026-07-08-m11c-chat-core-design.md`](superpowers/specs/2026-07-08-m11c-chat-core-design.md).
- **M11d — Default display modules:** independently-replaceable composer + message-card contribution modules; text enrichment (Markdown/HTML/images/links/emails, GM-gated, no embedded CSS); emotes; roll integration; internal doc links; SSRF-guarded server-side link previews.

### M12 · Minimal default modules
- Actor / scene browsers, generic actor / item sheets — built against the public API, each treated as an API bug report. (Chat panel superseded: the baseline chat display modules ship in **M11d**.)

### M12.5 · Backups + snapshot restore (gate precondition)
- Basic world backup (SQLite snapshot / per-world export) + restore path; minimal manual scheduling. Distinct from Phase-4 backup *automation*.
- Satisfies the dogfood-alpha gate's data-safety precondition.

**▶ Dogfood alpha gate** — backups (M12.5) must exist before real worlds accrue.

## Phase 2 — Full table
Combat tracker (initiative, hidden combatants, turn-event triggers; depends on M11 dice) → real asset pipeline (chunked upload, image conversion, tags, derived tags) + asset browser (regex / tag / dir search, preview / rename / move / tag) + bulk import/export → layout / theming completion (drag-resize, pop-out, multi / user themes, module styling modes) → vision / lighting / movement completion (photometric, darkvision / tremorsense / height; **per-actor/faction movement exemptions — flying/incorporeal ignore difficult terrain, deferred from M10g; needs movement-type tags on actors**) → token enrichment (aura / light / sound / VFX emitters, **trigger regions — mechanical/trigger effects built on the M10g region primitive: damage, condition application, scripted triggers on enter/arrest**, token-art, **generated token visuals (deferred from M10i) — a parametric compositor that frames existing actor art into a token: decorative border + shape-crop mask + background, distinct from the dynamic faction ring; a new additive `{kind:"generated"}` on the M10h `RenderVisual` union**, **per-token built-in fx (deferred from M10j) — condition-driven tint / desaturate / highlight + selection/faction/target highlight via a per-token Pixi `.filters` attach point on the M10h token `Container`; custom shader-filter seam stays Phase 3 VFX**, **emote / reaction overlays (deferred from M10j) — transient overlay above the token via a new ping-style `emote` aux frame + fading child**) → rollable tables (on the dice engine + document model), rich-text notes (on the document model), chat media linking (images; YouTube as thumbnail + external link only — no IFrame / Data API) → full default module suite → search consolidated into one milestone (single backend; no three-backend split).

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
