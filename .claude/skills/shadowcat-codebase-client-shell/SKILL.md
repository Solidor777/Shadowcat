---
name: shadowcat-codebase-client-shell
description: "Use when touching the Shadowcat UI shell: the contribution/Surface module architecture, AppContext, the hash router + entry views, i18n/locale, or the shell/panel modules (entry, core-ui, topbar, statusbar, settings). Covers src/client/{shell,ui-kit} + those src/modules. Invoke shadowcat-codebase-core first."
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
  contribute UI into named surfaces).
- `src/client/core/src/modules.ts` — `ModuleRegistry`; `services.ts` — `ServiceRegistry`;
  `topology.ts` — `reconcileTopology(...)` resolves `provides`/`requires` contracts (singleton
  loud-fail). Contract schemas in `wire.ts` (`ContractProvideSchema`).
- `src/client/ui-kit/src/Surface.svelte` — the `<Surface>` host that renders contributions for a
  surface id; `appContext.ts` — `AppContext`, `setAppContext`/`getAppContext`, `__APP_CONTEXT_KEY__`.
- `src/client/ui-kit/src/i18n.svelte.ts` — `t(key, params)`, `locale()`, the `i18n` adapter over
  core `i18n.ts` `I18n`; catalogs in `ui-kit/src/locales/`.
- `src/client/ui-kit/src/{sceneInteraction,actorSelection,tokenSelection}.*` — AppContext seams.
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
  `sceneInteraction.animateSamples(tokenId, samples, durationMs, startServerMs, moverVision)`, which
  forwards through `RenderEngine` to `TokenView`/`TokenAnimator` (position tween) and, when
  `moverVision` is present (mover only), the engine's `visionSweeps` fog-sweep playback (see
  `shadowcat-codebase-scene-rendering`).
- `src/client/shell/src/` — `App.svelte`, `main.ts`, `lib/` (hash router, api client, session,
  WorldSession controller, default-module wiring).
- `src/modules/{entry,core-ui,topbar,statusbar,settings,game-settings}/` — entry =
  `@shadowcat/module-entry` (login + world mgmt, behind `<Entry onEnterWorld>`); core-ui owns the
  layout grid + region surfaces into the singleton `root`; the rest each contribute one sidebar
  element. `game-settings` = `@shadowcat/module-game-settings` (GM-only): idempotently seeds +
  edits the three vision/lighting config-docs (`world-settings`/`light-gradation`/`vision-modes`,
  resolvers in `core/scene-docs.ts`) — world defaults, per-scene overrides (inherit = write `null`),
  gradation bands, vision modes.

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
