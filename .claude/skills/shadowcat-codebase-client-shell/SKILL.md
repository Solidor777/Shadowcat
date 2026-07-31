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

- `src/client/core/src/contributions.ts` — `Contribution`, `ContributionRegistry` (modules
  contribute UI into named surfaces). `Contribution.panel?` (M12a, replaced `tab`) is optional
  plain-data panel metadata (`icon`, `labelKey`, `gmOnly?`, `defaultPlacement`) the panel host
  renders; `labelKey` is an i18n key the HOST resolves (locale-reactive).
- **Panels replaced the tabbed sidebar** (M12a): the sidebar module and ui-kit
  `TabbedSurface` are DELETED. `@shadowcat/module-panels` provides the multi
  `shadowcat.panel` contract every panel module contributes into, hosts `PanelHost` in
  core-ui's singleton `shadowcat.surface:panel-host` region and the minimized-chips strip in
  statusbar's `shadowcat.surface:panel-dock`. Keep-mounted rule carries over: panels hide via
  CSS/slot adoption, never `{#if}`; hidden content reads `scrollHeight = 0` (module-chat's
  IntersectionObserver pattern still applies). Internals → [[shadowcat-codebase-panels]].
- `src/client/core/src/modules.ts` — `ModuleRegistry`; `services.ts` — `ServiceRegistry`;
  `topology.ts` — `reconcileTopology(...)` resolves `provides`/`requires` contracts (singleton
  loud-fail). Contract schemas in `wire.ts` (`ContractProvideSchema`).
- `src/client/ui-kit/src/Surface.svelte` — the `<Surface>` host that renders contributions for a
  surface id; `appContext.ts` — `AppContext`, `setAppContext`/`getAppContext`, `__APP_CONTEXT_KEY__`.
- `src/client/ui-kit/src/i18n.svelte.ts` — `t(key, params)`, `locale()`, the `i18n` adapter over
  core `i18n.ts` `I18n`; catalogs in `ui-kit/src/locales/`.
- `src/client/ui-kit/src/{sceneInteraction,actorSelection,tokenSelection}.*` — AppContext seams.
- **`AppContext.serverRole`** (`appContext.ts`) — the caller's SERVER tier (`"admin" | "user"`),
  distinct from the per-world `role`. Gates admin-only UI (the settings module's user manager).
  Derived in `App.svelte` from `/api/me` as `me?.server_role === "admin" ? "admin" : "user"`, so an
  absent or unrecognized value yields `"user"` — fail-closed. **It is COSMETIC**: the server
  re-checks every admin route through the `AdminUser` extractor, so a forged client gains nothing.
  Never gate an admin surface on the per-world `role` instead: `permission_context` maps
  `ServerRole::Admin → WorldRole::Gm`, so a world-role check is satisfied by any GM. All three
  `setAppContext` fixture sites default it to `"user"` so no existing test silently gains admin UI.
- `AppContext.pathfind` (`src/client/ui-kit/src/appContext.ts`) — correlated-request seam: issues a
  `Pathfind` frame via `WsClient.pathfind` and resolves with `PathResult` or rejects with
  `PathError`; wired through `WorldSession` and consumed by `scene-tools` measure-tool route mode.
- `WsClient.moveRequest(scene, tokenId, path) → Promise<MoveStream>` (`src/client/core/src/ws-client.ts`,
  M2 — `MoveExecuted` is FULLY RETIRED, server + Zod + client) — correlated-request mirror of
  `pathfind`: sends `MoveRequest`, resolves with the broadcast `MoveStream` when the matching
  `move_stream` frame arrives (mover's `request_id` correlates; the resolved value signals success
  only — it does NOT drive animation), rejects on `move_error` or timeout (default 10 s). Pure
  transport — no client-side movement logic. Keyed in the shared `pending` map alongside search and
  pathfind.
- `WsClient.onMoveStream(cb) -> unsubscribe` (M2) — the actual playback seam: fires for EVERY scene
  viewer (mover + observers) on every broadcast `MoveStream`, independent of the `moveRequest`
  promise. Listeners survive reconnects (not cleared by `failPending`).
- `AppContext.moveRequest` (`src/client/ui-kit/src/appContext.ts`) — AppContext seam wired through
  `WorldSession`; consumed by scene-tools measure-tool route-commit (sends `MoveRequest`, awaits the
  signal-only resolution, does NOT locally animate — the M10e-5 `TokenAnimator` plays back from the
  broadcast, not the promise). Optimistic dispatch + `collinearRuns` chaining were removed;
  route-commit is request-only.
- `onMoveStream` wiring (M2 Tasks 5-6, `worldSession.svelte.ts`): subscribes once at session start,
  **filters `stream.scene` against the active scene** (`this.#optimistic.query("scene")[0]?.id`)
  before forwarding — a room-wide `MoveStream` broadcast for a DIFFERENT scene must not animate a
  token or feed a fog sweep in the one currently rendered (cross-scene leak/flicker guard, mirrors
  the existing `toVisibility`/`toLighting` active-scene filter). On a match, calls
  `sceneInteraction.animateSamples(tokenId, samples, durationMs, startServerMs, () => ws.serverNow(), moverVision)`, which
  forwards through `RenderEngine` to `TokenView`/`TokenAnimator` (position tween) and, when
  `moverVision` is present (mover only), the engine's `visionSweeps` fog-sweep playback (see
  `shadowcat-codebase-scene-rendering`).
- **External-module loading (M13-1)** — `worldSession.svelte.ts`'s `#loadExternalModules(world,
  serverVersion)` runs after `Welcome` (`serverVersion` = `w.server_version`): fetches the world's
  enabled set (keyed on the install FOLDER id, `InstalledModuleInfo.id`, never manifest id), calls
  core `loadModules` (per-module-contained, non-throwing `ModuleLoadResult`), then activates. The
  shell serves ONE runtime instance of `svelte`/`@shadowcat/*` via `vite.config.ts` `RUNTIME_ENTRIES`
  + `preserveEntrySignatures:"strict"` + the `index.html` import map. GM management UI =
  `src/modules/settings/src/ModuleManager.svelte`. Full subsystem (server discovery/serving/enablement,
  engine-compat gate) → [[shadowcat-codebase-module-toolchain]].
- **Bounded + retried boot fetches (silent-hang-startup fix)** — `lib/api.ts`'s session/boot
  fetches (`getMe`, `getUiState`, `listWorlds`) each carry `AbortSignal.timeout(FETCH_TIMEOUT_MS)`
  (15s), so a hung backend rejects instead of leaving the fetch unsettled forever. `App.svelte`'s
  `boot()` wraps each of the three awaits in `withRetry` (3 attempts, flat delays) before
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
- `src/client/shell/src/` — `App.svelte`, `main.ts`, `lib/` (hash router, api client, session,
  WorldSession controller, default-module wiring). `sessionState.svelte.ts` owns the
  `ui_state` blob: `getPanelLayout(world)`/`setPanelLayout(world, blob)` (M12a, replaced
  activeTab) persist the per-world panel layout into `UiState.worlds[world].panelLayout` via
  the existing leading-edge-debounced PUT. The blob is OPAQUE to the shell — the panel host
  owns its shape/validation. **Leaf-key dirty tracking (fixes the same-user cross-session
  clobber, `docs/CLOSED_BUGS.md` "Server + client / ui-state persistence")**: a `dirty` structure
  (`Set<GlobalField>` + a `Map<worldId, Set<WorldKey>>`) tracks which individual FIELDS/KEYS
  changed since the last successful write — `global.locale`/`global.lastWorld` and
  `worlds.<id>.panelLayout`/`worlds.<id>.chatRead` each track independently, so two owners of the
  same slice (the panels module writing `panelLayout`, the chat module writing `chatRead` inside
  the same `worlds.<id>`) no longer clobber each other. `persist()`/`flushOnUnload()` build a
  `UiStatePatch` (`api.ts`) covering only those dirty leaves — never the whole slice, and never
  the whole `{global, worlds}` blob — clearing them before the write and re-marking on failure
  (both functions snapshot the dirty structure, clear it, attempt the write, and on rejection
  re-add every snapshotted field/key) so a retry doesn't lose the write. Server-side,
  `SqliteRepository::merge_ui_state` (`data/sqlite.rs`) merges the patch one level inside
  `worlds.<id>` and inside any other top-level object key — a leaf blob (`panelLayout`, etc.)
  still replaces wholesale, never deep-merged — in one transaction; the HTTP surface and size cap
  live in `http/routes.rs::put_ui_state`. The client never sends the whole `{global, worlds}` blob.
  Concurrent same-user sessions (two tabs) now contend only on the individual fields/keys both
  sessions actually write, instead of last-writer-wins on a whole slice or the whole blob.
- **Multi-scene / viewed-scene seams (M12d)** — `AppContext.viewedSceneId: string | null`
  (a live getter, `Table.svelte`: `get viewedSceneId() { return session.viewedSceneId; }` —
  NEVER destructure a snapshot of it), `AppContext.setGmViewedScene(id): void` (GM-only local
  roam; no-ops+warns for a non-GM), `AppContext.searchDocuments(query, opts, onUpdate) ->
  Promise<SubscriptionHandle>` (the M6c live-FTS subscription seam, newly exposed through
  `AppContext`/`WorldSession` — wraps `WsClient.subscribeSearch`, ephemeral/NOT
  reconnect-resilient), `AppContext.sceneSelection: SceneSelection`
  (`src/client/ui-kit/src/sceneSelection.svelte.ts` — a small stable-ref class, `configureSceneId`
  + `select(id)`, shell-constructed in `Table.svelte` like `panels`/`sheets`; distinct from BOTH
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
  call, found in M12d Task 8 review), "View" (`ctx.setGmViewedScene`), "Activate" (writes
  `activeScene` via `ctx.dispatchIntent` with the REAL current value as OCC `old`). Scenes have
  no `name` field — the browser labels rows by index + thumbnail, deliberately (not in the M12
  spec).
- AppContext seams (wired in `Table.svelte`): `uiState {getPanelLayout, setPanelLayout}`
  (narrow; the shell owns storage), `panels: PanelsApi & PanelsChipsView` — the shell
  constructs ONE `PanelsBridge` (`ui-kit/src/panelsBridge.svelte.ts`, `$state`-backed so
  pre-bind readers unfreeze at bind; details → [[shadowcat-codebase-panels]]) — and
  `chat: ChatApi {send, edit, delete}`
  (fire-and-forget over `WsClient.sendChatMessage`/`editChatMessage`/`deleteChatMessage` —
  these frames carry no correlation id; rejections are server-logged only, so composers
  pre-validate). `members` is now populated for EVERY role (chat name resolution; the
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
  (`world-settings`/`light-gradation`/`vision-modes`, resolvers in `core/scene-docs.ts`),
  plus `dice-settings` (M11d-2) and `chat-settings` (M11d-3: the `hyperlinks` +
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
- **Refactors across a callback boundary must preserve decision branches, not just await ordering**
  [[refactor-preserve-decision-branches]].
- UI packaging target: swappable entry package + per-element packages + thin shell
  [[ui-packaging-target]].

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §1 (client UI packaging) + §2 invariant 7 (framework-neutral API);
  `docs/PLAN.md` (M7/M8.5 milestones).
- Relationships:
  `graphify query "contribution registry surface appContext shell router i18n locale panel"`.
- History: [[m7-brainstorm]], [[m6b-modules-capabilities]].
