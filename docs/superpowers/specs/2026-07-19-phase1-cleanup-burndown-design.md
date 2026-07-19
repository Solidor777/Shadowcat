# Phase-1 Cleanup Burndown — Design Spec

**Status:** Approved design (brainstorm 2026-07-19). Sub-project 1 of the Phase-1-close
backlog clear. Base branch: `phase1-cleanup-burndown` off `main` (`48dfed9`).

## Context

Phase 1's final milestone (M13 · Nightfox) is complete on `main` (M13a→b→c→e→f all landed).
`OPEN_BUGS.md` is empty (0 confirmed defects). The remaining Phase-1-close work is the
`TODO.md` backlog (~50 live items). Per user directive, the rule is: **clear every item unless
it is blocked by an unbuilt future capability.** This spec covers the well-specified subset
(cleanups, tests, a11y, refactors, perf, now-unblocked fixes, and four small dogfood-polish
features). Larger net-new features that each carry a design fork are split into their own
follow-on sub-projects (see *Out of scope*).

### Standing design decisions (taken during brainstorm)

- **`set_pointer` removal semantics** → implement **true key removal** (removed key becomes
  *absent*, not `null`), now that the M13e merge engine has landed. `null ≠ absent` was the
  original complaint. Must not regress OCC pre-images or the merge-engine field model.
- **`Room::publish` Create-gate** → **by-design: initial placement stays GM/tool-privileged
  (unrestricted).** The create capability is already a privileged grant; unrestricted placement
  is normal authoring. Document the intent in code + ARCHITECTURE, close the item. No behavior
  change.
- **Four bucket-C features are in scope here** (small, self-contained, finish the playable
  alpha): speak-as composer picker, rich roll tooltips, unread badges, send/edit/delete failure
  surfacing. All other net-new bucket-C features → their own sub-projects.
- **Best-long-term-shape** is the default tie-breaker for any open shape question; escalate to
  the user only when the long-term shape is genuinely uncertain (per project design-fork rule).

## Goals

1. Reduce `TODO.md` to only genuinely-blocked items (each tagged with its blocking capability).
2. Land every actionable cleanup/test/a11y/refactor/perf/now-unblocked fix on `main`'s codebase.
3. Ship the four dogfood-polish features that finish the playable alpha.
4. Keep every gate green cross-platform (cargo test + clippy `--all-targets -D warnings`,
   `pnpm -r test`, typecheck, lint) on Windows/macOS/Linux.

## Non-goals

- The large bucket-C feature sub-projects (see *Out of scope*).
- Any item blocked by an unbuilt future capability (see *Deferred backlog* — these stay in
  `TODO.md`, retagged).
- Rewriting history or force-pushing. New commits only.

## Workstreams

Each item is a plan-task seed. `[S]`/`[M]`/`[L]` = rough size. `[sec]` = touches a
security/secrecy seam → **mandatory two-reviewer security buddy-check**. `[perf]` items:
measure first where the TODO says "inert until measured", then implement the best-long-term
shape (user directive: do the perf work; measure where measurement is the honest gate).

### WS-A — Server hygiene & perf (Rust)

- **A1** `[S]` Batch the four `get_or_create` config/actor `query_documents` in `ws/room.rs`
  into one `WHERE doc_type IN (...)` (halve cold-room DB round-trips).
- **A2** `[M][perf]` `engine_as::<T>()` (`scene/mod.rs`) full `serde_json::from_value` per call
  on vision/lighting/pathfinding hot paths. Profile; add a per-entity cached decode (or a
  borrowed deserialize). Best-long-term shape: cache the decoded struct per entity, invalidated
  on the entity's `engine` mutation.
- **A3** `[S]` A* search-window edge: add `tracing::debug!` at window-edge leg failures
  (`scene/pathfinding.rs`) for future tuning.
- **A4** `[S]` Unrestricted-mode fog sweep: gate `mover_vision` in `execute_move`
  (`ws/room.rs`) on **mover role**, not `MovementRestriction::Unrestricted`, so a non-GM mover
  in an Unrestricted scene still gets a progressive sweep. `[sec]` (vision path — verify no
  observer leak).
- **A5** `[M][perf]` Cache the per-`(user, scene)` `visible_cells`/`player_lit_mask` for the
  M10e-4 movement gate; reuse the last egress-computed mask instead of recomputing per move.
  Invalidate on the inputs that change the mask (token move, wall/light/vision-mode mutation).
  `[sec]` (the mask IS the secrecy gate — a stale cache must fail toward *recompute*, never
  toward a wider mask).
- **A6** `[S]` `list_members`: `ORDER BY u.username COLLATE NOCASE` (case-insensitive roster).
- **A7** `[M]` Bundle link-preview deps (`preview_client`/`preview_cache`/`preview_rate`) into a
  `LinkPreviewDeps` struct; thread through `handle_send_message`/`handle_edit_message` (~40 call
  sites); remove the `#[allow(clippy::too_many_arguments)]`.

### WS-B — Server correctness / now-unblocked (Rust)

- **B1** `[M][sec]` `command::set_pointer` **true key removal**: an Update that removes a key
  removes it (absent), not writes `null`. Verify interaction with the field-level OCC `old`
  pre-image and the M13e merge base (a removed key must round-trip through merge as absent, not
  null). Tests: remove→absent, OCC on removed key, merge base with a removed key.
- **B2** `[M][sec]` Singleton doc-type **create-gate**: a server-side registry of singleton
  doc_types (`faction-registry`, `condition-registry`, `world-settings`, `CHAT_SETTINGS_DOC_TYPE`,
  `DICE_SETTINGS_DOC_TYPE`) consulted at the `apply_intent` Create chokepoint; reject a second doc
  of a singleton doc_type in a world (fail-closed). Replaces the client seed-guards as the
  *guarantee* (they stay as UX). Complements the already-landed deterministic-lowest-UUID
  resolution.
- **B3** `[S]` `Room::publish` Create-gate: document initial placement as intentionally
  GM/tool-privileged/unrestricted (code comment + ARCHITECTURE invariant-6 note). Close the
  design-question item. No behavior change.

### WS-C — Server scene-vision features, now unblocked by scene bounds (Rust) `[sec]`

> This workstream touches the fog/vision **secrecy gate**. Both items get a mini-design in the
> plan + a mandatory two-reviewer security buddy-check. If either design fork proves large at
> planning time, promote it to its own sub-project.

- **C1** `[M][sec]` Edge-projected, `blocksLight`-occludable **environment light**: implement the
  ambient edge-light projection in `player_lit_mask` now that `scene.system.bounds{width,height}`
  exists (M10f-0). A `blocksLight`-sealed interior must be darkened by the ambient term (placed-
  light occlusion already works). Lighting stays cosmetic — fog remains the secrecy gate.
- **C2** `[M][sec]` Wall-less scene **full intrascene vision**: a wall-less scene reveals vision
  covering its own bounded extent (via `scene.system.bounds`) instead of the degenerate
  viewpoint±margin box. Leak-safe: bounded strictly to the scene's own extent (never a global
  `mode:"all"`, which would cross-scene-leak — the M12d `viewedSceneId` guard class).

### WS-D — Module-toolchain hardening (Rust + build) (M13-1 deferrals)

- **D1** `[S]` `scan_installed_modules`: wrap the blocking `std::fs` I/O in `spawn_blocking` (it
  now runs on the per-WS-connect Welcome path).
- **D2** `[S]` `welcome_capability_requirements`: dedup `(path_prefix, caps)` entries (choose a
  dedup-key strategy for `CapabilityRequirement`).
- **D3** `[S]` `modules.e2e.test.ts` fixture: make `engines.shadowcat` track the running server
  version (or a permissive `*`) so a version bump past `0.1` doesn't fail with a misleading 422.
- **D4** `[M]` Build-time guard: fail the build if a `from "svelte/..."` specifier not enumerated
  in `vite.config.ts` `RUNTIME_ENTRIES` appears (protects the single-instance-runtime invariant).
- **D5** `[M]` `ModuleRegistry.activate()` register() lifecycle contract (now reachable, M13b+):
  best-long-term shape = wrap the `register()` call with a `removeModule(id)` cleanup sweep on
  catch (roll back `hooks.on`/`services.provide`/`use`/`contributions.contribute` from a module
  that throws mid-`register()`).
- **D6** `[S]` Module-authoring guide (M13-1 Task 17 doc): call out that a package **subpath**
  import (`@shadowcat/core/x`) is an unresolvable bare specifier under the exact-match import map.

### WS-E — Client refactors (TS/Svelte)

- **E1** `[L]` **ActorsPanel god-component split**: extract `VisualKindEditor.svelte` (owning
  `AnimSourceState`/`FaceRowState`/`buildVisual`); extract the face-swap palette; fold
  `faceRowComplete` + `buildVisual`'s inline completeness check into a shared
  `animSourceComplete(anim)` helper; add the missing test (a linked token whose
  `overrides.visual` is a `faces` union + an active `system.face` face-swap).
- **E2** `[M]` **Shared menu primitive**: extract the WAI-ARIA menu keyboard/focus model
  (arrows/Home/End/Escape/Tab + wrap-around `focusItem`) from `LauncherMenu`/`PanelMenu` into a
  ui-kit primitive; refactor both onto it.
- **E3** `[S]` `buildTokenFromActor` w/h: retain `w/h = cellSize` seeding **solely as the
  documented dangling-link fallback** (a comment stating its only consumer is `resolveTokenBox`'s
  missing-actor path); no lazy recompute (YAGNI). Best-long-term shape chosen: explicit,
  documented fallback over a second derivation path.

### WS-F — Client a11y + panels polish (TS/Svelte)

- **F1** `[S]` `ToolRail` `.controls select/input` coarse-pointer sizing (`@media (pointer:
  coarse)` ~44px).
- **F2** `[M]` ui-kit **shared input-height token/rule** for text/number/checkbox `<input>`
  coarse-pointer sizing (systemic; not a per-component media query). Covers the pre-existing
  `GameSettingsPanel`/`SystemTreeEditor` gap.
- **F3** `[M]` `DockviewEngine.apply()`: sync an **already-floating** panel's live re-drag/resize
  back into the tree `Rect` (creation is already handled; re-drag currently drifts).
- **F4** `[M]` **Whole-group drag** transfers (`PanelTransfer.panelId === null`): translate into
  per-tab dock ops instead of vetoing, re-enabling the group-drag gesture.
- **F5** `[S]` Narrow `PanelHost.svelte`'s `PanelsBridgeLike` cast (runtime `typeof bridge.bind`
  guard or a narrower `AppContext.panels` type).
- **F6** `[S]` `DockChips.svelte`: i18n fallback for a raw untranslated panel id (match
  `PanelHost.describeOp`'s aria-live fallback).
- **F7** `[S][stretch]` `DockviewEngine.apply()` finer content-independent group-identity diff
  (avoid tear-down/recreate on first-tab reorder). Optional — inert cosmetic churn today; include
  only if it lands cheaply.

### WS-G — Client perf (TS)

- **G1** `[M][perf]` `pixi-backend.ts` `captureFog`: cache/reuse the two cross-fade
  `RenderTexture`s across ticks (recreate only on resize or fog-input change) instead of a
  full-screen recapture per `setVisibilityBlend`.
- **G2** `[M][perf]` Chat message-list: virtualize (render window) + narrow the reactive
  subscription so the full history isn't re-parsed/re-sorted on every document mutation.
- **G3** `[S][perf]` Route-preview re-requests: fixed debounce + seq-guard on waypoint change
  (leading-edge + max-staleness per `debounce-leading-edge-not-trailing-rearm` if a fast-drag
  profile shows chattiness).

### WS-H — Client small cleanups + tests (TS)

- **H1** `[M]` `module-factions` GM seed: deterministic `faction-registry` id + dedupe-on-conflict
  (multi-GM first-entry race). Pairs with WS-B2's server create-gate.
- **H2** `[S]` Game-settings scene picker: display a human-readable scene name. **Plan-time
  check:** confirm whether a `scene` doc carries a name/label field; if not, add one + its editor.
- **H3** `[S]` M10e-6 cleanup bundle: `point_segment_distance` degenerate threshold
  (`f64::EPSILON` → geometry-scale ~1e-10); `pathfinding.rs` `use` decls to top-of-file;
  `grid.test.ts` explicit `dmin=2→3` alternating assert; `Stage.svelte` inner `scene` →
  `activeSceneDoc`; `ws-client.test.ts` stop re-serializing a parsed object; `PendingResult`
  alias for the `SearchPage|PathResult` union.
- **H4** `[S]` `panels.spec.ts`: locate tool buttons via `data-testid="tool-{id}"`, not the
  styling class.
- **H5** `[S]` `sizeClass.svelte.ts` teardown test (+ the paired i18n teardown-test gap).
- **H6** `[S]` `controller.test.ts` boot-race: assert full `compact.order` sequence equality.
- **H7** `[S]` Browser e2e: assert the scene **background** renders (`scene.system.background` →
  sprite).

### WS-I — Bucket-C dogfood-polish features

- **I1** `[M]` **Speak-as composer picker** (`actor_owner`): the picker UI only — wire field,
  storage, and card rendering already support it. (Token-instance attribution stays a separate
  sub-project.)
- **I2** `[M]` **Rich roll tooltips**: a popover with the full per-die table, replacing the
  native `title` attribute on the inline chip.
- **I3** `[M]` **Unread badges / notification pips** on the chat tab.
- **I4** `[L][sec]` **Send/edit/delete failure surfacing**: add a protocol-level correlation-id +
  reason channel so a server rejection (flood limit, permission) is surfaced to the sender
  instead of being silent. Server + client. `[sec]` — the reason channel must not leak
  authorization detail to an unauthorized sender (fail-closed, generic reasons where the detail
  is sensitive).

### WS-J — Docs / tracking sync (final step)

- **J1** `[S]` `PLAN.md`: add the missing **M13e DONE** roadmap entry (templates/`base`/3-way
  merge engine).
- **J2** `[S]` Prune all `RESOLVED` entries from `TODO.md`.
- **J3** `[S]` Rewrite `TODO.md` to only the **Deferred backlog** items below, each tagged with
  its blocking capability. Keep the reference notes (the `axum_test` dot-segment gotcha; the
  module-requirements-advisory design note; the module-toolchain scope exclusions).
- **J4** `[S]` `PLAN.md`: record the Phase-1 cleanup-burndown entry and list the remaining
  bucket-C feature sub-projects as the last Phase-1-close work.
- **J5** `[S]` **Reviewed skill-update gate** (mandatory, project CLAUDE.md §1): update every
  affected `shadowcat-codebase-*` skill for any seam/invariant/gotcha changed by this
  sub-project; dispatch `shadowcat-spec-reviewer` to confirm each skill diff. Blocks completion.

## Deferred backlog (stays in `TODO.md`, retagged — blocked by an unbuilt capability)

| Item | Blocked by |
|---|---|
| `explored_fog` purge on deletion | world/scene/user **deletion** (unbuilt) |
| `MoveOutcome.cost` / `los_smooth` cost comparability | per-turn **movement-budget** system (Phase-2 combat) |
| Token rotation shortest-delta lerp | **rotation authoring** (unbuilt) |
| `reconcileTopology` version/`provides` mismatch | **module management** / hard topology enforcement |
| `LauncherMenu` open-while-`metaMap`-mutates focus recovery | **live** module management |
| Singleton multi-provider conflict policy | a real **2nd provider** of a singleton contract |
| Capability **version negotiation** | multiple providers at differing versions |
| Tier-ladder `margin_offset` uniqueness guard | a wire-facing **Tier construction** path |
| `DieKind::Faces` `ReplaceDie` out-of-range guard (pt 2) | **recalc-from-chat** wire (closes with that sub-project) |
| GM see-as-player **MoveStream** preview | **see-as-preview** feature buildout |
| Popout `onWillDrop` subscription | **multi-panel popout** groups gesture |
| `DockviewEngine#toDropSite` fallback exhaustiveness | real pointer-gesture QA (unsimulable under jsdom) → manual QA |
| `FakeEngine` `PanelMenu` | a bespoke-fallback caller needing it (production never reaches it) |

## Out of scope — follow-on feature sub-projects (each own brainstorm → spec → plan)

Built after Sub-project 1, one design pass each (user: build ALL of bucket C):

1. **Recalc-from-chat** — persist `spec`/`raws` on `RollEmbed` (persistence + secrecy fork);
   closes the `DieKind::Faces` `ReplaceDie` guard at the same boundary.
2. **Link-preview extensions** — server-fetch-cache-as-asset **image** pipeline + async
   post-publish enrichment (`WriteOrigin` path) + **shared preview cache** + **oEmbed** provider
   embeds (user opted both edge items in; oEmbed carries SSRF/privacy surface → threat-model it).
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Dice-notation grammar growth** — math fns (floor/ceil/round/abs/min/max) + crit-event /
   tier-ladder notation syntax (also opens the Tier-uniqueness guard).
5. **Per-channel / per-message dice-settings overrides** — needs a channel model.
6. **In-body doc-link chat segment** (`Segment::DocLink`) — server producer + authoring
   affordance.
7. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` composer/token-context UX + lift
   the fail-closed ingest rejection.

## Testing & verification

- **TDD per item** (project rule). Each fix ships with a test that fails before / passes after.
- **Full gate, cross-platform:** `cargo test --all-targets` + `cargo clippy --all-targets -- -D
  warnings`; `pnpm -r test`; typecheck (esbuild strips types — typecheck is a *separate* gate per
  `vitrest-skips-typecheck-in-sdd`); lint. No debug artifacts in committed code.
- **Mandatory two-reviewer security buddy-check** on every `[sec]` item: A4, A5, B1, B2, C1, C2,
  I4. Vision/secrecy items verify no observer leak and fail-closed behavior.
- **`pnpm -r test`** (not a single-package filter) for any shared wire-schema change
  (`shared-wire-schema-change-needs-full-repo-test`).

## Sequencing note for the plan

WS-J (docs) is the final step. WS-C (`[sec]` vision) and WS-B1/B2 + WS-I4 (`[sec]`) are the
highest-risk; schedule their buddy-checks with margin. WS-E1 (ActorsPanel `[L]`) and WS-E2
(shared menu) are the largest refactors. Everything else is independent and parallelizable.
