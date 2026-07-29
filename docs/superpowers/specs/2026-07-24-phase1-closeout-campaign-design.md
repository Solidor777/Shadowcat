# Phase-1 Close-Out Campaign — Design

**Date:** 2026-07-24
**Status:** Draft — awaiting user review
**Goal:** `docs/TODO.md`, `docs/OPEN_BUGS.md`, and `docs/POST_WORK_FINDINGS.md` are empty or
rationale-only before Phase 2 begins.

## Campaign rules (user-set, iron-clad)

1. **No deferrals.** "Do it later" is not an outcome. Every open item is resolved in this
   campaign or closed with explicit recorded rationale approved here.
2. **Discovered TODO → resolve.** Newly found issues join the campaign; they are not logged
   and left.
3. **Needs discussion → flag to the user.** Genuine judgment calls stop for a decision;
   nothing is silently descoped.
4. **Discovered bug → fix.**
5. At any design fork, implement the best long-term shape; churn is not a concern.

## Scope decisions already made (user-answered during design)

| Decision | Answer |
|---|---|
| MoveStream live cross-animation concurrency | **In** — feature phase of this campaign |
| Singleton conflict policy + capability version negotiation | **Design + build now** (semantics validated against synthetic fixtures; recorded caveat: no real second provider yet) |
| Polish bucket (grid-kind tag, code-span shortcodes, write-quiesce, scope pin, atomic restore) | **All in** |
| Popout cross-window drops | **Full multi-panel popout group support now** (Phase-2 layout item pulled forward) |
| 7 follow-on feature sub-projects | **Folded into this campaign** (Stage 2; each keeps its own focused design cycle) |
| POST_WORK_FINDINGS | **Triage + resolve in-campaign**; file ends empty or rationale-only |
| Lenient-mode corner-ε over-inclusion | **Keep fail-safe**; close with rationale (security over rare cosmetic reject) |
| Route-vs-gate footprint asymmetry | **Build footprint-aware authoritative blocking now** (shared predicate, never-fork) |
| Replay redaction point-in-time fidelity | **Accept + close with rationale** (replay is recovery, not audit; tightened policy retro-redacting is the safe direction) |

Also resolved during design: hex-grid server movement is **already DONE** (shipped
`GridShape` + parity battery; `docs/TODO.md`'s entry is stale doc-drift, fixed in Stage 3).
`docs/OPEN_BUGS.md` is already empty.

## Campaign structure

- **Stage 0 — Findings triage formalization.** Confirm the triage table (§Stage 0) against
  the code, close the no-work entries with rationale, and verify the two "verify-then-fix"
  items. Output: every findings entry mapped to a Stage-1 work item or a rationale closure.
- **Stage 1 — Phases A–G.** The cleanup + hardening + pulled-forward feature work, fully
  designed in this spec. Each phase: `writing-plans` → `mainline-plan-execution` → review →
  merge → doc/skill sync.
- **Stage 2 — Feature sub-projects H1–H7.** Each gets its own focused brainstorm → spec →
  plan cycle inside the campaign (their open design surface — oEmbed threat model, channel
  model — cannot be responsibly pre-designed here). Scope and constraints pinned in §Stage 2.
- **Stage 3 — Final sync + push.** Tracking files sparse, PLAN.md updated, skills through the
  reviewed gate, CI green across the three-OS matrix, push.

Ordering rationale: cheap high-value hardening first (A), the deletion feature that unblocks
several purges second (B), then correctness/UX phases, the biggest engine feature (G) last in
Stage 1 so finish-up value lands early.

---

## Stage 0 — POST_WORK_FINDINGS triage table

**Close with rationale (no work):** dark-scene movement freeze (working as designed);
`core:delete` GM-only (documented); grants-to-`None` (intentional); CI lagged-WS
(already resolved); M8c-2 token re-audit (resolved); M12b toolrail order (resolved);
M13-0 `/engine` staleness (resolved by Task 7; residual `apply_command` gate becomes work item
A8); update-to-deleted-doc replay drop (harmless, final-state convergent); multi-leg
alternating parity per-leg-greedy (spec-compliant, display-only); corner-ε over-inclusion
(user decision above); replay redaction (user decision above); token-move gate hydration race
(investigated, unreachable); route-stricter-than-gate (superseded — gate adopts footprint
model, work item D4).

**Verify-then-fix:** M12 singleton-config dedup for the seed race (did M12 land it? if not,
build it — work item E9); M10e-2 edge-projected environment light "logged to TODO.md" pointer
(entry absent from TODO.md — doc drift; the work itself is item D5).

**Become work items:** listed in their phases below with a `(findings)` tag.

---

## Stage 1 phase designs

### Phase A — Security, limits & server hygiene

- **A1. Shared auth throttle.** One hand-rolled in-memory keyed limiter (no new dependency):
  fixed-window with small burst allowance, keyed per-IP and per-identity (username for
  `/api/login`, account id for `/api/invites/accept`). Both endpoints keep their
  full-Argon2 anti-enumeration behavior; the limiter sits in front and returns 429 uniformly.
- **A2. Invite GC.** Sweep `world_invites` rows with `expires_at` well past inside the
  existing `spawn_session_sweep`; no new timer.
- **A3. `TokenEngine.x/y` ingress validation.** Finiteness + magnitude bound in
  `validate_engine`, sharing `MAX_GATE_WALK_COORD` (one symbol, anti-drift test) — closes the
  GM-write/Create-path vs move-gate admissibility fork.
- **A4. Six `unwrap_or(100.0)` survivors.** `navmesh_for`, `region_field`, `player_lit_mask`,
  `visible_cells`, `visible_cells_cached`, `enrich_vision_explored`: absent-scene becomes an
  explicit `Option`/`Result` the caller must handle. `scene_grid_sizes` remains the one
  intentional defaulting source.
- **A5. `ScenePing` guard.** Require the scene doc to exist in this world AND sender holds
  `cap::READ` on it — admits the token-less spectator, refuses unseen scenes.
- **A6. `handle_send_message` world-scope pin** on the actor doc.
- **A7. Backup/restore atomicity.** Restore: write to temp paths, atomic-rename swap for
  `world.db` and the assets dir (rename dance; no window where the destination is neither
  old nor new). Backup: brief write-quiesce gate (asset writes + DB writes barrier) held
  across `VACUUM INTO` + assets copy, closing the replace-race metadata/bytes skew.
- **A8. `apply_command` `/engine` normalization gate** *(findings)* — parity with
  `apply_intent`'s gate now, not when a future undo/replay caller trips it (never-fork).
- **A9. Dice construction guards.** `RecalcOp::ReplaceDie` gains the Faces-vs-Numeric gate +
  `natural` range check; tier ladders get a validated construction boundary (unique
  `margin_offset` enforced where a ladder enters `classify`/recalc, `Result` not panic). Any
  future wire path (H4's tier-ladder syntax, H1's recalc exposure) is then safe by
  construction.
- **A10. Dice test/display fixes** *(findings)*: Rust-side test for `RollOutcome` missing
  `labeled_consts`; labeled-const display honors enclosing `Neg` (`-3[dex]` shows `-3`).

### Phase B — World & user deletion

- **B1. `DELETE /api/worlds/{id}`** — allowed for server admin or that world's GM.
  Order: (1) evict the live room (close frames; drop from registry + scene/navmesh caches),
  (2) one DB transaction: delete the world row (FK cascades cover members/documents/assets
  rows/events/invites; FTS delete triggers fire — verify triggers fire under FK cascade at
  plan time, else explicit FTS deletes in the same tx) + `DELETE FROM explored_fog WHERE
  world_id = ?` with a new `world_id` index, (3) after commit, delete the world's asset
  directory from disk (a crash orphans files, never leaves a live world missing files —
  matches the commit-row-before-file convention).
- **B2. `DELETE /api/users/{id}`** — admin-gated, last-admin guard (mirror of the last-GM
  guard). Migration: `assets.created_by` → `ON DELETE SET NULL` (today: no action — the
  delete would FK-fail). `documents.owner_id`/`events.author_id`/invite columns already
  `SET NULL`; memberships cascade. Revoke the user's sessions and kick live connections in
  the same operation.
- **B3. Scene-delete fog purge.** `DELETE FROM explored_fog WHERE scene_id = ?` inside the
  scene-document `Operation::Delete` transaction.
- **B4. `add_member` transaction.** Resolve+insert in one tx (safe the moment B2 ships).
- **B5. Minimal UI affordances.** World delete (type-the-name confirm) in the entry
  world-management view; user list + delete in the admin surface where `POST /api/users`
  creation lives. No new management screens.

### Phase C — Egress ownership unification

- **C1.** Egress (`filter_properties`, `collect_hidden`, `filter_command`, document routes)
  resolves `is_owner` via the same `effective_owner` as the write path, backed by an
  in-memory resolved-owner cache (token → linked-actor owner) maintained on document
  mutation alongside the room's existing side-tables — no per-recipient pool query on the
  hot path.
- **C2.** Owner floor grants `cap::READ` at egress: a document you can write is a document
  you receive (closes the write-but-never-receive asymmetry).

### Phase D — Movement & scene correctness

**Amendment (Phase D-α close-out, 2026-07-29): Phase D split in two.** Exploration for D4 found
three additional items and one already-shipped item, so Phase D executed as two spec cycles rather
than one:

- **Phase D-α — movement authority & secrecy** (`docs/superpowers/specs/2026-07-25-phase-d-alpha-movement-authority-secrecy-design.md`,
  branch `phase-d-alpha-movement-authority`): **D10, D9, D8, D4**, in that order — one coherent,
  security-sensitive restructure of server-side movement authority. Three items were added, none in
  this spec's original table: **D10** (wall secrecy axis — `move_walls` gains a `viewer` parameter,
  mirroring `region_field`'s two-value contract, closing a route-shape leak of `gm_only` walls);
  **D9** (player moves become request-only — the standing server-authoritative-movement rule was
  violated by the select-tool drag path; closing it deletes `Room::publish`'s non-GM traversal gate
  entirely rather than reconciling it with `execute_move`); **D8** (GM gate-exemption unification —
  `execute_move` had drifted into enforcing walls/impassable/arrest against GMs, a regression
  against the original M9 design spec's GM-bypass grant; the spec wins, so the enforcement is
  removed). **D5** (edge-projected environment light) was found already shipped (`513aef8`,
  `e1156ae`, 2026-07-19) and moved out to D-β as verify-then-close rather than re-executed.
  A plan-level buddy check on the resulting 11-task plan found 5 Critical / 14 Important / 14 Minor
  findings, all folded in before execution began (`docs/superpowers/sdd/2026-07-25-phase-d-alpha-movement-authority-secrecy/progress.md`).
  D-α executed via SDD (11 tasks, per-task two-reviewer gate, reviewed skill-update gate on
  `shadowcat-codebase-scene-rendering`).
- **Phase D-β — movement & scene correctness (later spec)**: the remaining **D3, D1+D2, D7, D6,
  D5** (D5 as verify-then-close only). Not yet executed as of this amendment.

- **D1. Cost unification.** `move_exec` threads the diagonal rule + per-step parity so
  `MoveOutcome.cost` equals the router's preview cost — one cost model, parity-tested
  (never-fork applied to cost semantics).
- **D2. `los_smooth` exact cost.** Recompute exact per-span cost for smoothed chords
  (replaces the conservative pre-smoothing value); with D1, preview = execution everywhere.
- **D3. Hex bugs** *(findings)*: resolve `ResolvedScene.bounds` to its declared grid-unit
  semantic (M10f-0) — `vision::bound_for_scene` converts instead of consuming raw; hex
  extent conversion via `GridShape` (a `w×h`-cell hex scene spans `w·√3·size` × 
  `(1.5·h+0.5)·size`, not `w·size` × `h·size`); step-to-world-distance factor comes from
  `GridShape` (fixes the 1.73× hex continuous-weighted preview cost); add the non-GM
  hex+continuous `pathfind` test with a real mask through `clip_to_visible_mask`.
- **D4. Footprint-aware authoritative gate** (user decision). The movement gate adopts the
  router's footprint-clearance predicate — one shared symbol, parity-tested both directions;
  route-admissible ⇔ gate-admissible. GM exemption unchanged.
- **D5. Edge-projected environment light** *(findings; promised for "when scene dimensions
  exist" — they do, M10f-0 bounds)*. Environment light projects from the scene-bounds edges,
  occluded by `blocksLight` walls, replacing the flat-ambient deviation; day/night
  color+intensity unchanged.
- **D6. Lighting render polish** *(findings)*: per-cell radial gradients replacing the single
  BlurFilter soften; darkvision desaturation via masked ColorMatrixFilter over the scene
  layers (wire already carries the faithful per-cell hint; client-render-only).
- **D7. `explored_fog` grid-kind tag.** Blob header gains version+kind; a live grid-kind
  switch re-indexes that scene's blobs transactionally through world-space cell-center
  round-trip (old kind → world coords → new kind) — preserves exploration rather than
  clearing it.

### Phase E — Client authoring & UX

- **E1. Rotation authoring.** Select-tool rotation affordance in scene-tools (rotate handle +
  modifier-scroll), GM/owner-gated, OCC `Update` to the token engine rotation with the
  raw-stored-`old` convention, snap honoring the scene snap toggle.
- **E2. Shortest-arc rotation lerp.** `TokenAnimator` lerps `((b−a+540)%360)−180` with a
  wrap-aware ε-settle.
- **E3. Scene background authoring.** Asset picker on `SceneBrowserPanel`'s thumbnail slot →
  OCC `Update` to `/engine/background` (raw-old convention) + Playwright e2e asserting the
  background sprite renders.
- **E4. Scene-scoped `tokenSelection`.** Per-scene selection map in `worldSession`; selection
  survives GM roam-away-and-back; gated with `pnpm -r test`.
- **E5. World-defaults editor completes `WorldSceneDefaults`** *(findings)*: world-level
  authoring for `losRestriction`/`fog`/`observerVision`/`partialCellLeniency`/`environment`.
- **E6. Offline-intent flush ordering** *(findings)*: gate `#flushOfflineQueue` on an
  "onWelcome settled" promise.
- **E7. Vision-mode validation warning** *(findings)*: GM-facing diagnostic when a mode entry
  is dropped for a missing `illuminationFloor`.
- **E8. Caption text-size token** *(findings; the M12 re-audit point arrived)*: add a
  font-size scale token (`--text-sm`-class) and apply to the asset-tile filename.
- **E9. Singleton-config seed dedup** *(findings; verify-then-fix)*: if M12 didn't land it,
  enforce singleton uniqueness for world config-docs server-side (per-doc_type constraint),
  making the seed race harmless by construction.
- **E10. `@shadowcat/formula` resolver-error helper** for `evaluate.ts` + `template.ts`
  (`graph.ts`'s trampoline-entangled catch stays separate, as recorded).
- **E11. Nightfox sheet UX** *(findings; lands in the Nightfox repo, user pushes)*:
  pointer-events/long-press reorder for StatTable (iOS touch gap — cross-platform directive);
  StatRow invalid numeric input resets to last valid value + visible error indicator.
- **E12. Shortcodes skip code spans.** Pre-parse replacement becomes backtick-span-aware
  (CommonMark backtick-string rules).
- **E13. `EFFECT_DOC_TYPE` promotion** *(findings)*: promote beside `ITEM_DOC_TYPE` in
  `scene-docs.ts`.

### Phase F — Module system & panels

- **F1. `reconcileTopology` hard enforcement.** Flag version and `provides`/`requires`
  mismatches for modules present on both sides; mismatches are loud reconcile failures with
  actionable messages, not silent reconciles.
- **F2. `LauncherMenu` mutation safety.** Focus-recovery when the focused entry vanishes from
  `metaMap` (fall back to first item) + pinning test.
- **F3. Full multi-panel popout groups** (Phase-2 item pulled forward, user decision).
  Open popout windows get `onWillDrop` subscriptions; cross-window drops are accepted and
  translated through `applyOp` (veto/classify pipeline intact); `#poppedOutGroupPanels`
  becomes real multi-panel tracking with window-close accounting for every panel in the
  group.
- **F4. `FakeEngine` PanelMenu.** Mount the existing menu in `FakeEngine`'s tab strip so a
  bespoke-fallback panel can leave a zone through UI.
- **F5. Real-pointer drag e2e.** Playwright suite driving real drag gestures over
  `toDropSite` classification (edge vs center vs tab-strip index) including popout drops —
  closes the manual-QA residual and the M12a automation gap *(findings)*.
- **F6. Singleton conflict policy.** `provides` gains optional integer `priority`; highest
  wins; ties keep the deterministic loud-fail; a world-level explicit winner override
  (config doc keyed by contract id) beats priority. Validated against synthetic fixture
  modules; recorded caveat: semantics unvalidated by a real second provider until one exists.
- **F7. Capability version negotiation.** `requires` matches providers by semver range
  against the provider's declared contract version; no match = loud fail naming both sides.
- **F8. Context-bearing module activation phase** *(findings; API bug report)*: a
  session-scoped activation hook so stateful modules (panels) stop repeating the
  construct-at-mount + bridge dance. Designed against the public-API rule; pre-freeze work.
- **F9. External-module i18n seam** *(findings)*: public registration of i18n keys into the
  shell catalog, replacing Nightfox's built-in fallback-map workaround.
- **F10. External-module browser e2e harness** *(findings)*: extend the Playwright infra to
  load an external module and cover the author→equip→toggle→revert flow in a real browser.

### Phase G — MoveStream live cross-animation concurrency

**Design: event-driven re-clip, not a per-tick server loop.** Once a move executes, its
position and vision trajectories are deterministic (moves are never redirected mid-flight;
arrest only truncates). Exact concurrency semantics therefore need no polling:

- When move B executes while move A is in flight, recompute each affected recipient's clip of
  A's *remaining* samples against that recipient's now-known time-parameterized vision
  trajectory, and send a `MoveStreamAmend` frame for the in-flight stream (ts-rs + Zod
  mirror).
- Re-clip triggers: (1) any move execute while others are in flight; (2) vision-affecting doc
  mutations (wall/light changes) during flight.
- Client `TokenAnimator` accepts mid-playback sample-set amendments (it already handles
  gap/occlusion + catch-up).
- Secrecy invariant unchanged: a recipient only receives samples their authoritative vision
  admits at send time; amendments only widen from provably-visible. A wholly-occluded move
  still produces zero frames for that recipient until an amendment makes it visible.
- If a future feature makes trajectories non-deterministic mid-flight, the amend seam is the
  same one a tick loop would feed — the design degrades gracefully to the TODO's literal
  suggestion.

---

## Stage 2 — Feature sub-projects (each: own brainstorm → spec → plan, in-campaign)

| # | Sub-project | Pinned scope + constraints |
|---|---|---|
| H1 | Recalc-from-chat | Persist `spec`/`raws` on `RollEmbed` (persistence + secrecy fork). A9's construction guards already make the recalc boundary safe; this wires the UX. |
| H2 | Link-preview extensions | Server-fetch-cache-as-asset image pipeline + async post-publish enrichment (`WriteOrigin` path) + shared preview cache + oEmbed. **oEmbed carries SSRF/privacy surface — threat-model it in its design cycle** (the existing GuardedResolver/IP-blocklist discipline is the floor). |
| H3 | Per-world export/import | World-scoped row subset preserving cross-FK referential integrity + shared asset references. B's deletion cascade map is the input inventory. |
| H4 | Dice-notation grammar growth | Math fns (floor/ceil/round/abs/min/max) + crit-event/tier-ladder syntax. A9's validated tier-ladder construction is the ingress guard. |
| H5 | Per-channel/per-message dice-settings overrides | Needs a channel model — that model is the design cycle's main question. |
| H6 | In-body doc-link chat segment | `Segment::DocLink` server producer + client authoring affordance. |
| H7 | Speak-as-token-instance | Composer/token-context UX + lift the fail-closed `ActorOwnerRef::TokenInstance` ingest rejection together. |

Order H1→H7 as listed (dice/chat items cluster early while that context is warm; H3 benefits
from B's cascade inventory). Reorder freely if a design cycle surfaces a dependency.

---

## Stage 3 — Final sync

- `docs/TODO.md`: every entry resolved or closed; stale hex entry corrected to DONE with
  pointers; movement-budget entries closed by D1/D2.
- `docs/OPEN_BUGS.md`: stays empty (bugs found mid-campaign are fixed in their phase).
- `docs/POST_WORK_FINDINGS.md`: empty or rationale-only closures per Stage 0.
- `docs/PLAN.md`: campaign recorded; Phase-2 list updated (popout groups moved here).
- `shadowcat-codebase-*` skills updated per phase through the reviewed skill-update gate.
- Each phase is a milestone-scale unit: push after its merge, `gh run watch` on the
  three-OS matrix; full campaign completion is the final gate.

## Testing strategy

- Every never-fork change (A3, D1, D4) ships an anti-drift/parity test exercising both paths
  through the shared symbol, mutation-checked (change one side, test must fail).
- Server: `cargo test` + fmt/clippy per phase; client: `pnpm -r test` + typecheck + lint
  (full-repo gates for client changes — sibling fixtures break silently otherwise).
- Playwright suites (E3, F5, F10) run headed locally, headless in CI.
- Security-sensitive phases (A, B, C, G) get the two-reviewer pair
  (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`) at their final review; the rest get
  the standard `mainline-plan-execution` single dispatched whole-branch review.

## Success criteria

1. All Stage-1 work items merged, CI green on all three OSes, pushed.
2. All Stage-2 sub-projects shipped through their own cycles.
3. The three tracking files sparse per Stage 3; no entry says "later" without user-approved
   rationale recorded in this spec's decision table.
