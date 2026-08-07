---
name: shadowcat-codebase-client-shell
description: "Use when touching the Shadowcat UI shell: the contribution/Surface module architecture, Contribution.panel metadata, AppContext (incl. the chat, uiState.panelLayout, panels/PanelsBridge, and multi-scene viewedSceneId/setGmViewedScene/searchDocuments/sceneSelection seams), the hash router + entry views, i18n/locale, or the shell/UI modules (entry, core-ui, topbar, statusbar, settings, scene-browser). Covers src/client/{shell,ui-kit} + those src/modules. For the panel-manager internals (module-panels, engines, layout tree) invoke shadowcat-codebase-panels; for the render-engine consumption of viewedSceneId invoke shadowcat-codebase-scene-rendering. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Client Shell & UI Modules

Orientation for the SPA shell, the UI-as-modules contribution architecture, and i18n.

## Purpose

The browser UI is layered: a thin app **shell** bootstraps routing/session/AppContext and wires
the default module set; in-game UI is contributed by `src/modules/*` packages into named
**surfaces** via a `provides`/`requires` contract system; entry views (login/world management) are
plain-routed, not contributions. i18n is a framework-neutral core with a thin Svelte adapter.

## Key files & seams

- `Contribution`, `ContributionRegistry` (modules
  contribute UI into named surfaces). `Contribution.panel?` (replaced `tab`) is optional
  plain-data panel metadata (`icon`, `labelKey`, `gmOnly?`, `defaultPlacement`) the panel host
  renders; `labelKey` is an i18n key the HOST resolves (locale-reactive).
- **Panels replaced the tabbed sidebar**: the sidebar module and ui-kit
  `TabbedSurface` are DELETED. `@shadowcat/module-panels` provides the multi
  `shadowcat.panel` contract every panel module contributes into, hosts `PanelHost` in
  core-ui's singleton `shadowcat.surface:panel-host` region and the minimized-chips strip in
  statusbar's `shadowcat.surface:panel-dock`. Keep-mounted rule carries over: panels hide via
  CSS/slot adoption, never `{#if}`; hidden content reads `scrollHeight = 0` (module-chat's
  IntersectionObserver pattern still applies). Internals → [[shadowcat-codebase-panels]].
- `ModuleRegistry`; `ServiceRegistry`;
  `reconcileTopology(...)` resolves `provides`/`requires` contracts (singleton
  loud-fail). Contract schemas: `ContractProvideSchema`.
- `<Surface>` is the host that renders contributions for a
  surface id; `AppContext`, `setAppContext`/`getAppContext`, `__APP_CONTEXT_KEY__`.
- `t(key, params)`, `locale()`, the `i18n` adapter over
  core's `I18n`; catalogs in `ui-kit/src/locales/`.
- `src/client/ui-kit/src/{sceneInteraction,actorSelection,tokenSelection}.*` — AppContext seams.
- **The three selection classes share a shape but NOT their repeat-set reactivity.** All of
  `ActorSelection`, `TokenSelection`, `SceneSelection` are stable instances mutated in place (never
  reassigned) so the AppContext-captured reference stays valid, and none of them PRUNE an id whose
  document is later deleted — a stale id stays selected until something clears it, so every consumer
  MUST resolve against the current store and handle the miss itself. That is an obligation, not an
  observation: today's consumers all check, but not uniformly — scene-tools' place tool aborts on a
  miss while its measure tool substitutes defaults (`eng?.x ?? 0`, a 0.4 footprint), which is a
  silently-wrong measurement rather than a no-op. They diverge on what re-selecting the
  CURRENT value does: `ActorSelection`/`SceneSelection` are `$state`-backed scalars, so `select(same)`
  is a no-op for reactivity (`$state`'s default `===`), while `TokenSelection` is `SvelteSet`-backed
  and `set()` clears-then-re-adds, so passing back an identical id list still re-triggers every
  reader of `.ids`/`.has`/`.size`. The one exception is empty→empty: `SvelteSet.clear()` early-returns
  without bumping its version when already empty, so that case alone is a genuine no-op. Do not
  reason from "they're siblings" to "they behave alike" — an effect keyed off `TokenSelection` runs
  on repeat-sets that an `ActorSelection`-keyed one skips.
- **`AppContext.serverRole`** — the caller's SERVER tier (`"admin" | "user"`),
  distinct from the per-world `role`. Gates admin-only UI (the settings module's user manager).
  Derived in `App` from `/api/me` as `me?.server_role === "admin" ? "admin" : "user"`, so an
  absent or unrecognized value yields `"user"` — fail-closed. **It is COSMETIC**: the server
  re-checks every admin route through the `AdminUser` extractor, so a forged client gains nothing.
  Never gate an admin surface on the per-world `role` instead: `permission_context` maps
  `ServerRole::Admin → WorldRole::Gm`, so a world-role check is satisfied by any GM. All three
  `setAppContext` fixture sites default it to `"user"` so no existing test silently gains admin UI.
- `AppContext.pathfind` — correlated-request seam: issues a
  `Pathfind` frame via `WsClient.pathfind` and resolves with `PathResult` or rejects with
  `PathError`; wired through `WorldSession` and consumed by `scene-tools` measure-tool route mode.
- `WsClient.moveRequest(scene, tokenId, path) → Promise<MoveStream>` (`MoveExecuted`
  is FULLY RETIRED, server + Zod + client) — correlated-request mirror of
  `pathfind`: sends `MoveRequest`, resolves with the broadcast `MoveStream` when the matching
  `move_stream` frame arrives (mover's `request_id` correlates; the resolved value signals success
  only — it does NOT drive animation), rejects on `move_error` or timeout (default 10 s). Pure
  transport — no client-side movement logic. Keyed in the shared `pending` map alongside search and
  pathfind.
- `WsClient.onMoveStream(cb) -> unsubscribe` — the actual playback seam: fires for EVERY scene
  viewer (mover + observers) on every broadcast `MoveStream`, independent of the `moveRequest`
  promise. Listeners survive reconnects (not cleared by `failPending`).
- `AppContext.moveRequest` — AppContext seam wired through
  `WorldSession`; consumed by scene-tools measure-tool route-commit (sends `MoveRequest`, awaits the
  signal-only resolution, does NOT locally animate — the `TokenAnimator` plays back from the
  broadcast, not the promise). Optimistic dispatch + `collinearRuns` chaining were removed;
  route-commit is request-only.
- `onMoveStream` wiring (`WorldSession.enter`): subscribes once at session start,
  **filters `stream.scene` against the active scene** (`this.#optimistic.query("scene")[0]?.id`)
  before forwarding — a room-wide `MoveStream` broadcast for a DIFFERENT scene must not animate a
  token or feed a fog sweep in the one currently rendered (cross-scene leak/flicker guard, mirrors
  the existing `toVisibility`/`toLighting` active-scene filter). On a match, calls
  `sceneInteraction.animateSamples(tokenId, samples, durationMs, startServerMs, () => ws.serverNow(), moverVision)`, which
  forwards through `RenderEngine` to `TokenView`/`TokenAnimator` (position tween) and, when
  `moverVision` is present (mover only), the engine's `visionSweeps` fog-sweep playback (see
  `shadowcat-codebase-scene-rendering`).
- **External-module loading** — `WorldSession`'s `#loadExternalModules(world,
  serverVersion)` runs after `Welcome` (`serverVersion` = `w.server_version`): fetches the world's
  enabled set (keyed on the install FOLDER id, `InstalledModuleInfo.id`, never manifest id), calls
  core `loadModules` (per-module-contained, non-throwing `ModuleLoadResult`), then activates. The
  shell serves ONE runtime instance of `svelte`/`@shadowcat/*` via `RUNTIME_ENTRIES`
  + `preserveEntrySignatures:"strict"` + the `index.html` import map. GM management UI =
  `ModuleManager`. Full subsystem (server discovery/serving/enablement,
  engine-compat gate) → [[shadowcat-codebase-module-toolchain]].
- **`boot()` resolves the world route-first, not `lastWorld`-first (silent-hang-startup fix)** —
  `App`'s `boot()` reads `currentRoute()` once, AFTER the `getMe`/`getUiState` awaits and
  BEFORE both the `withRetry(() => listWorlds())` await and consulting `ui.global.lastWorld` (a
  hash change during the `listWorlds` await is ignored — see TODO).
  The rule lives in one pure, directly-testable helper, `resolveBootWorld(route, lastWorld,
  worlds)`: a world route (`#/world/<id>`) always wins — `lastWorld` is
  NOT consulted at all while a world route is present, even if it would resolve to a different,
  still-valid world; `lastWorld` seeds ONLY a bare/non-world load. A route's world id absent from
  `listWorlds()` (deleted/revoked) clears `lastWorld` ONLY if `lastWorld` is ALSO stale — a dead
  deep link must never wipe an otherwise-valid `lastWorld` reference — then lets `boot()` fall
  back to the entry/worlds-list route. Entering the
  resolved id still goes through `enterWorld(worldId)`, which itself calls `setLastWorld` +
  `navigate` — `lastWorld` write semantics are unchanged. Root-caused via a captured Playwright
  network trace: under the shared-account parallel e2e suite, a reload's `boot()` ignoring the URL
  restored whichever world a DIFFERENT concurrent worker entered last — a real product defect
  (a deep-linked reload in production would teleport away from its own URL the same way), not an
  e2e-only artifact. See CLOSED_BUGS, the "Client / silent-hang startup paths" entry.
- **Bounded + retried boot fetches (silent-hang-startup fix)** — `FETCH_TIMEOUT_MS`
  (15s) covers every fetch in its module, not only the session/boot trio: `getMe`, `getUiState`,
  `listWorlds`, `postJson` (login/logout), and `putUiState` (including the unload keepalive PUT)
  all carry `AbortSignal.timeout(FETCH_TIMEOUT_MS)`, so a hung backend rejects instead of leaving
  any of them unsettled forever. `App.boot()` wraps each of the three awaits in `withRetry` (3 attempts, flat delays) before
  degrading to the login/worlds route — a transient non-2xx or connection reset during startup no
  longer permanently strands the SPA on that fallback route with no retry.
- **`WorldSession`'s activation latch is split, and the split order is load-bearing
  (silent-hang-startup fix)** — a single `#bootstrapped` boolean used to latch BEFORE
  `await #modules.activate()`, so a failed/hung first activation (e.g. a manifest dependency
  cycle) cached "done" for the session's life: reconnect Welcomes short-circuited, `role` was set,
  but every Surface stayed empty. It is now two fields: `#modulesAdded` (latches once per
  session — re-adding modules would duplicate registrations) and `#activated` (latches only on a
  successful `activate()`, reverted to `false` in the `catch` on a thrown activation, so the NEXT
  Welcome retries instead of caching the failure). **`#activated` is still set to `true`
  SYNCHRONOUSLY, before the `activate()` await** — this is the one part of the old single-latch
  behavior deliberately preserved: same-tick concurrent Welcomes re-enter `#onWelcome`, and
  setting `#activated` only after the await (e.g. in a `.then()`) would let a second Welcome
  arriving mid-activation see `#activated === false` and call `activate()` again, double-
  activating. Any future change to this seam must keep the synchronous pre-await set — do not
  "simplify" it to an after-await assignment.
  **`leave()` is a PARTIAL teardown, and the latches are what makes that dangerous.** It stops and
  drops the `WsClient` and resets `state`/`role`/`world`/`#gmViewedScene`, but does NOT clear
  `store`/`documents`, module registrations, or EITHER latch. That is correct only because
  `App.leaveWorld` discards the instance and constructs a fresh `WorldSession`. Reuse one
  across `leave()` → `enter()` and it carries the PREVIOUS world's state into the new one: `store`/
  `documents`, module registrations, and the `contributions` registry all survive (nothing clears
  them — `ContributionRegistry` drops entries only via a contribution's own dispose or
  `removeModule`), while both latches stay set so the next Welcome skips activation. Surfaces then
  render the previous world's contributions — stale cross-world content, NOT an empty screen, which
  is the harder failure to notice. Treat `WorldSession` as single-use per world entry. (Distinct
  from what the latch split itself guards, which is a FAILED first activation being cached.)
  **Two more boundaries worth knowing before relying on `enter()`:** it resolves when the connect
  ATTEMPT SETTLES — `WsClient.open` catches a failed `connect` and schedules a reconnect instead of
  rejecting — so resolution implies neither an open transport nor a usable world; Welcome, module
  activation, the member fetch, and external module loading all happen afterward and are not
  awaited. And `#onWelcome` contains its failures asymmetrically: the member fetch has its own inner
  catch (logged, non-blocking), whereas an activation throw reverts `#activated` and RE-throws,
  skipping EVERY later step in that Welcome — external-module loading, the member fetch, topology
  reconcile, scene re-establishment, the GM first-scene seed — so a failed activation also costs
  that Welcome's scene re-subscription, not just its modules. An outer catch means the method
  itself never rejects, so neither failure surfaces to a caller.
- The shell package — `App`, the `main` entry module, and its `lib/` directory (hash router, api client, session,
  WorldSession controller, default-module wiring). The `sessionState` module owns the
  `ui_state` blob: `getPanelLayout(world)`/`setPanelLayout(world, blob)` (replaced
  activeTab) persist the per-world panel layout into `UiState.worlds[world].panelLayout` via
  the existing leading-edge-debounced PUT. The blob is OPAQUE to the shell — the panel host
  owns its shape/validation. **Leaf-key dirty tracking (fixes the same-user cross-session
  clobber — see CLOSED_BUGS, the "Server + client / ui-state persistence" entry)**: a `dirty` structure
  (`Set<GlobalField>` + a `Map<worldId, Set<WorldKey>>`) tracks which individual FIELDS/KEYS
  changed since the last successful write — `global.locale`/`global.lastWorld` and
  `worlds.<id>.panelLayout`/`worlds.<id>.chatRead` each track independently, so two owners of the
  same slice (the panels module writing `panelLayout`, the chat module writing `chatRead` inside
  the same `worlds.<id>`) no longer clobber each other. `persist()`/`flushOnUnload()` build a
  `UiStatePatch` covering only those dirty leaves — never the whole slice, and never
  the whole `{global, worlds}` blob — clearing them before the write and re-marking on failure
  (both functions snapshot the dirty structure, clear it, attempt the write, and on rejection
  re-add every snapshotted field/key) so a retry doesn't lose the write. Server-side,
  `SqliteRepository::merge_ui_state` merges the patch one level inside
  `worlds.<id>` and inside any other top-level object key — a leaf blob (`panelLayout`, etc.)
  still replaces wholesale, never deep-merged — in one transaction; the HTTP surface and size cap
  live in `http::routes::put_ui_state`. The client never sends the whole `{global, worlds}` blob.
  Concurrent same-user sessions (two tabs) now contend only on the individual fields/keys both
  sessions actually write, instead of last-writer-wins on a whole slice or the whole blob.
- **Multi-scene / viewed-scene seams** — `AppContext.viewedSceneId: string | null`
  (a live getter, `Table`: `get viewedSceneId() { return session.viewedSceneId; }` —
  NEVER destructure a snapshot of it), `AppContext.setGmViewedScene(id): void` (GM-only local
  roam; no-ops+warns for a non-GM), `AppContext.searchDocuments(query, opts, onUpdate) ->
  Promise<SubscriptionHandle>` (the live-FTS subscription seam, newly exposed through
  `AppContext`/`WorldSession` — wraps `WsClient.subscribeSearch`, ephemeral/NOT
  reconnect-resilient), `AppContext.sceneSelection: SceneSelection`
  (a small stable-ref class, `configureSceneId`
  + `select(id)`, shell-constructed in `Table` like `panels`/`sheets`; distinct from BOTH
  `viewedSceneId`/`activeScene` — configuring a scene's per-scene settings never moves any
  camera/render target). `WorldSession.viewedSceneId` (getter) resolves via `resolveViewedScene`
  (`@shadowcat/core`): a resolvable `gmViewedScene` (GM local roam, `WorldSession`-private
  `#gmViewedScene` `$state`) → a resolvable `world-settings.activeScene` (players follow) → the
  first scene (legacy fallback) → `null` only when no scene exists at all. See
  `shadowcat-codebase-scene-rendering` for how the render engine consumes this seam.
  `@shadowcat/module-scene-browser` (GM-only panel, `order: 6`) is the authoring surface: list +
  background thumbnails, create, "Configure" (deep-links the EXISTING `game-settings` panel's
  per-scene section via `ctx.sceneSelection.select(id)` +
  `ctx.panels.open("game-settings:panel")` — the exact `"<module>:panel"` contribution-id
  convention every `PANEL_CONTRACT` registration uses; a bare module id silently no-ops the open
  call), "View" (`ctx.setGmViewedScene`), "Activate" (writes
  `activeScene` via `ctx.dispatchIntent` with the REAL current value as OCC `old`). Scenes have
  no `name` field — the browser labels rows by index + thumbnail, deliberately.
- AppContext seams (wired in `Table`): `uiState {getPanelLayout, setPanelLayout}`
  (narrow; the shell owns storage), `panels: PanelsApi & PanelsChipsView` — the shell
  constructs ONE `PanelsBridge` (`$state`-backed so
  pre-bind readers unfreeze at bind; details → [[shadowcat-codebase-panels]]) — and
  `chat: ChatApi {send, edit, delete}`
  (over `WsClient.sendChatMessage`/`editChatMessage`/`deleteChatMessage`. These frames DO carry a
  `request_id` and `chatPending` is keyed by it: a `chat_error` correlates back and REJECTS the
  caller's promise with the server's player-presentable reason, which the composer surfaces.
  SUCCESS is the asymmetric case — the broadcast Event echo carries no `request_id`, so nothing
  acknowledges an accepted op and the 15s timer RESOLVES on silence. Exactly three settle paths:
  that timer, a `chat_error` reject, and a `failPending` reject — reached from BOTH a disconnect
  and an explicit `stop()` (`WorldSession.leave()`, and the `evicted` frame handler). Details →
  [[shadowcat-codebase-chat]]). `members` is now populated for EVERY role (chat name resolution; the
  roster endpoint was widened from GM-only), not just GM.
- `src/modules/{entry,core-ui,panels,stage,topbar,statusbar,settings,game-settings,scene-browser,
  chat,chat-composer,chat-card}/` — entry = `@shadowcat/module-entry` (login + world mgmt, behind
  `<Entry onEnterWorld>`); core-ui owns the layout grid + region surfaces into the singleton
  `root` (its main region hosts `shadowcat.surface:panel-host`; the grid cell carries
  `min-height: 0` + `overflow: hidden` — the growth cap that keeps tall content scrolling
  INSIDE panels instead of blowing the 1fr track past 100vh); the grid's compact/expanded
  switch is keyed SOLELY off `sizeClass()` (48rem, `ui-kit`'s single breakpoint axis) — no
  media query; `grid-template-rows` reserves a fixed `2rem` statusbar row in both states.
  panels = the panel manager ([[shadowcat-codebase-panels]]); stage = the canvas stage well
  (inviolable — never docked/floated/minimized); the panel modules each contribute one
  `shadowcat.panel`; defaults are launcher-closed for every panel except chat (docked right
  by default); game-settings gmOnly. `topbar` = `@shadowcat/module-topbar`: hosts
  `LauncherMenu` (open/close any registered panel by id via `AppContext.panels.toggle`,
  `launcher-item-{panelId}` testids, a11y menu + focus management) + `Presence` (member
  roster) + a standing settings-entry button that toggles `settings:panel` through the same
  `AppContext.panels` seam — imports NOTHING from `@shadowcat/module-panels` (seam boundary by
  design: topbar's package.json declares no module-panels dependency and the launcher talks to
  panels only through `AppContext.panels`; NOT lint-enforced — the repo's sole
  `no-restricted-imports` rule covers `dockview-core` only, and .svelte files are unlinted).
  Below 48rem the
  topbar drops the world-name label and the scene-tools `ToolRail` collapses from a vertical
  side rail into a horizontal bottom strip (`sizeClass()`-driven, same axis).
  `game-settings` = `@shadowcat/module-game-settings` (GM-only): idempotently seeds + edits
  the world's singleton config-docs — the vision/lighting trio
  (`world-settings`/`light-gradation`/`vision-modes`, resolved by
  `resolveSceneSettings`/`resolveGradation`/`resolveVisionModes`),
  plus `dice-settings` and `chat-settings` (the `hyperlinks` +
  `link_previews` tri-state toggles). Each section uses the same reactive-seed + real-OCC-
  pre-image `set()` idiom. The chat/dice server resolvers + segments are covered by
  `shadowcat-codebase-chat`/`-dice`.

## Hard invariants

- **A value put into `setContext`/AppContext must be a stable, in-place-mutated ref** (e.g. a
  `SvelteMap`), not a reassigned `$state`, or consumers hold a stale snapshot
  [[svelte-context-stable-ref]].
- **Contribute/activate before any `await` that gates the host mount** — an async-populated
  contribution Surface paints blank until activation runs; the minimal fix touches only the
  diverging path [[refactor-async-contribution-paint-timing]].
- **In-game elements communicate ONLY through seams** (module contracts, `ContributionRegistry`,
  `<Surface>`, AppContext, render-layer API) — never import one another or the shell directly
  (ARCHITECTURE §1, §2 invariant 7).
- **Entry views are plain-routed, not contributions; surfaces are in-world only.**
- **A config-doc seed `$effect` must be reactive (`createSubscriber` + `subscribe()`)** — contribution
  panels mount during `#onWelcome` BEFORE the resync stream populates the store, so a one-shot
  non-reactive seed either fails-to-seed (role not yet set) or double-seeds (store still empty). Mirror
  `FactionsPanel`/`ConditionsPanel`/`GameSettingsPanel`: GM-gate, `subscribe()` inside the effect,
  per-doc-type `length === 0` guard, single `seeded` latch [[contribution-seed-reactive-before-resync]].

## Gotchas

- **i18n MUST stay framework-neutral** — the core `I18n` is Svelte-free; the Svelte `t`/`locale`
  adapter wraps it via `createSubscriber`. Don't pull a Svelte i18n lib into core.
  **There is NO cross-locale fallback:** a key missing from the ACTIVE locale's catalog renders as
  the raw key string, even when another loaded locale defines it. A partial translation therefore
  ships visible key text rather than English, so a new key must land in every shipped catalog.
- **`setAppContextForTest` does not emulate optimistic behavior** —
  `documents` defaults to `over.documents ?? over.store ?? new
  DocumentStore()`, so a test overriding only `store` gets that SAME plain store as `documents`.
  Predicted-op overlay and rollback-on-reject are absent; reads through `documents` are plain
  authoritative reads. In production the two are INDEPENDENT siblings fed the same `applyCommand`.
  A test asserting optimistic semantics must supply its own `documents`, or it is asserting
  nothing. (Same fixture-fidelity class as the nightfox `t: (k) => k` gotcha.)
- **`WorldSession.canEdit` is an affordance mirror, and it diverges from server authz in BOTH
  directions** — treat it as "which controls to show", never as the
  authority. It can over-permit: the `role === "gm"` short-circuit returns `true` unconditionally and
  never consults `doc.permissions.gm_role`, while the server's GM bypass is CONDITIONAL — a doc
  carrying `gm_role: Some(role)` floors even a GM to ordinary `DocRole` resolution
  (`data::permission::effective_role`/`data::permission::resolve_access`). It can also over-restrict: the Welcome
  union mixes GM-authored `world_cap_requirements` with module-declared manifest requirements, and
  the server does NOT reject a write merely because a module declared a requirement on that path.
  **The two halves are unobservable today for UNRELATED reasons, and arm on unrelated triggers —
  do not give them a shared bound.** The over-permit is neutralized by the server: `SqliteRepository::apply_intent`
  re-checks independently, and `gm_role` is currently written only by chat-message construction
  (`chat::build_message_doc`), whose `message` doc_type the server rejects ordinary client Updates to outright
  regardless of role (`SqliteRepository::apply_intent`; only the server-set `WriteOrigin::ServerMessageRevision`
  bypasses it, which no wire message can name). It arms if `gm_role` is ever set on another
  doc_type. The over-restrict is NOT neutralized by that re-check at all — hiding a control means
  the user never reaches `apply_intent` — it is merely unobservable while no enabled installed
  module declares `requirements`, since the Welcome union then contributes nothing beyond the
  GM-authored record. It arms when an ENABLED, ENGINE-COMPATIBLE module declares a non-empty
  `requirements` — compatibility is re-checked on every Welcome, not just at enable time, so a
  module that has gone stale stops publishing — with no `gm_role` involvement whatsoever
  (`ws::conn::welcome_capability_requirements`, whose own doc marks the union ADVISORY ONLY,
  vs. `SqliteRepository::apply_intent`'s enforcement reading only `world_cap_requirements`).
- **`listWorldMembers` is FORKED — two implementations of one endpoint, already diverged.** The
  shell's `api` module's `listWorldMembers` goes through `getJson`: it has a request timeout, does NOT
  `encodeURIComponent` the world id, and raises status-only errors. Core's public
  `user-rest` module's `listWorldMembers` (re-exported from `@shadowcat/core`'s public entry) encodes the id and surfaces the server's error
  text, but has no timeout. The shell's `members` seam calls its own copy
  (`WorldSession`'s), so the two can drift further with nothing failing. This is the
  never-fork-a-decision class from `shadowcat-codebase-core`; do not add a third caller to either
  copy without collapsing them. Fix logged in TODO.
- **Refactors across a callback boundary must preserve decision branches, not just await ordering**
  [[refactor-preserve-decision-branches]].
- UI packaging target: swappable entry package + per-element packages + thin shell
  [[ui-packaging-target]].

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §1 (client UI packaging) + §2 invariant 7 (framework-neutral API);
  `docs/PLAN.md`.
- Relationships:
  `graphify query "contribution registry surface appContext shell router i18n locale panel"`.
- History: [[m7-brainstorm]], [[m6b-modules-capabilities]].
