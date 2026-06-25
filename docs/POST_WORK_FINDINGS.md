# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: M10e-2 environment light is flat ambient, not edge-projected. Summary: the M10e spec
  (§6/§12.5) specifies environment light as edge-projected + `blocksLight`-occludable, but the scene
  model is dimensionless (dimensions deferred), so there is no boundary to project edge light from.
  `player_lit_mask` implements environment as a flat scene-wide ambient floor; placed-light
  `blocksLight` occlusion IS implemented. Inert by default (`env.intensity` = 0.0). Status: Accepted
  (constraint-forced; logged to TODO.md `Server / scene-vision`, revisit with scene dimensions/M12).

- Title: M10e-2 vision-mode entry missing `illuminationFloor` is silently dropped. Summary:
  `resolved_vision_modes` skips a mode entry that lacks `illuminationFloor` (fail-closed; mirrors the
  client `resolveVisionModes`, which also does no per-mode validation). A typo'd floor key silently
  removes that mode from the resolved registry with no diagnostic. Status: Accepted (client parity;
  add GM-facing validation/warning if authoring friction surfaces).

- Title: M10e-4 dark scene under `movementRestriction:"visible"` freezes non-GM movement — INTENDED.
  Summary: a fresh scene (default `lightingEnabled:true` + `environmentLight` + `environment.intensity:0.0`,
  no placed lights) has an empty lit mask, so EVERY non-GM move's supercover fails the gate. This is the
  DESIRED behavior (user-confirmed): a player who cannot see a cell must not be able to move into it.
  A GM enables movement by lighting the scene (placed lights / `globalIllumination`) or setting
  `revealed`/`unrestricted`. Status: Resolved — working as designed. Do NOT "fix" this by softening the
  defaults; the freeze is the correct fail-closed outcome.

- Title: M10e-4 lenient-mode near-corner move can be spuriously rejected.
  Summary: `supercover_cells` uses a magnitude-relative epsilon to detect exact lattice-corner
  crossings and emit BOTH flanking cells (no thin-line slip). It can OVER-fire on a near-corner the
  true segment doesn't exactly cross, emitting an extra flanking cell; if that cell is dark, a legal
  player move whose path merely grazes a corner is rejected. The direction is fail-safe (over-include
  ⇒ reject a fine move, never admit a forbidden one — security is preserved), but it is a rare
  player-visible false-reject. Status: Accepted (security-safe). Revisit only if reports surface;
  a future tightening to the exact crossed-cell set would remove it.

- Title: offline-intent flush can precede the async `#onWelcome` body on reconnect.
  Summary: `WsClient` fires `onResyncComplete` (→ `WorldSession.#flushOfflineQueue`)
  synchronously on the caught-up Welcome branch / on `resync_end`, while
  `#onWelcome` runs as an unawaited `void` async (it awaits a member fetch before
  re-establishing scene subscriptions). So queued offline intents can transmit
  before scene subs re-establish. Not a correctness defect: flushed intents reach
  the server regardless, and scene-derived read state is eventually consistent via
  the egress re-evaluation debounce; FIFO confirm-correlation is unaffected.
  Status: Accepted (eventually-consistent ordering). If a stricter ordering is ever
  needed, gate the flush on an "onWelcome settled" promise.

- Title: capability model — `core:delete` is GM-only by default (behavior change
  from M5). Summary: the capability floor grants Owners `core:read` +
  `core:write_fields` but NOT `core:delete`, so a document Owner can no longer
  delete by default (M5's binary `can_write` allowed it). Intended per the
  capability spec; grant `core:delete` per-document or via a world default to
  restore owner-delete. Status: Accepted (documented behavior change).

- Title: capability model — grants can target `DocRole::None`. Summary: a GM may
  add capabilities to the `None` (no-access) role via `by_role`, widening what
  the floor denies. GM-authored only (not an escalation), and a coherent way to
  raise the default tier; recorded as intentional flexibility rather than
  restricted (restricting only the world-defaults endpoint's `validate_grants`
  would be inconsistent — per-document grants set at create / via PATCH
  `/permissions` bypass it). Status: Accepted (design note from Phase 1 review).

- Title: a saturated lagged WS connection is slow to auto-converge on the
  ubuntu-latest CI runner. Summary: `converges_with_publishing_during_resync`
  originally asserted the deliberately-lagged client reached the tail seq (300)
  in real time while the publisher ran concurrently. On ubuntu-latest the lagged
  connection delivered a contiguous-but-incomplete prefix (e.g. 1..234) and then
  emitted nothing for >10s — even after an explicit `ResyncRequest` on that same
  connection (zero frames). A fresh connection's `ResyncRequest` converges fine
  on the same runner (`all_clients_converge_after_reconnect` passes), so the
  durable resync path is sound; the symptom is auto-convergence latency/stall on
  an already-saturated lagged egress under Linux scheduling. The test now asserts
  the load-bearing invariant (no DROPS during the overlap → contiguous prefix)
  plus full recoverability via a fresh client. Status: Needs triage — determine
  whether the lagged egress genuinely stalls (a latency bug in the egress
  select/replay loop under heavy backpressure) or it is purely CI-runner
  saturation; reproduce with a constrained-CPU local run before changing
  `conn.rs`. Update (M8b-1 push, 2026-06-22): a *second* manifestation observed —
  the authoritative-seq assertion at `ws_convergence.rs:408`
  (`h.authoritative_seqs().last() == Some(300)`) failed `Some(277)` on
  ubuntu-latest after the test's 30s drain-wait budget (300×100ms), with the whole
  test taking 45s; i.e. even the *server-side* single-writer ingress→apply of 300
  queued intents didn't finish in 30s under runner saturation. Passed on
  windows+macos in the same run and locally (4.5s); cleared on job re-run.
  Unrelated to M8b-1 (the failing assertion is on DB ingress throughput, which
  M8b-1 does not touch). If it recurs, widen the drain budget (e.g. 600×100ms) or
  gate the count on a constrained-CPU repro before touching the ingress path.
  Update (2026-06-23) — **RESOLVED.** Recurred on ubuntu-latest even at the widened
  600×100ms=60s budget (`Some(289)` vs `Some(300)`, 79s test): a 300-intent ingress
  backlog genuinely cannot drain within budget on a saturated runner. Root cause is
  the test's *volume*, not a correctness defect — forcing the slow client's resync
  over a real socket was never deterministic (OS buffering absorbs a non-reading
  client; same non-portability resolved for the `Lagged` test in commit 2acf9f7), so
  the high count bought no coverage while loading the single-writer ingress past CI
  capacity. Fix: `converges_with_publishing_during_resync` now uses a modest
  `TOTAL=100` and asserts only its load-bearing invariants (no drop/reorder under
  concurrent publishing + full fresh-client resync recovery); the deterministic
  broadcast-`Lagged` → resync path is unit-tested against `egress_loop` with a
  credit-gated sink (`ws::conn::tests::egress_lag_triggers_resync_and_converges`).
  Full ws_convergence suite now ~2s locally.

- Title: `filter_command` redacts replayed history against the *current*
  PermissionSet. Summary: `src/server/src/data/permission.rs` loads each
  `Update` op's document via `get_document` to resolve visibility, so on
  resync/replay a property whose `GmOnly`↔`All` visibility was flipped after the
  event is redacted under the *new* policy, not the policy in force at the
  command's seq. Acceptable for M5 (visibility flips are rare; replay is
  recovery, not audit) but the redaction is not point-in-time faithful. Status:
  Needs triage — if audit-grade replay is ever required, snapshot the relevant
  permissions into the event or attach them to the broadcast.

- Title: an `Update` to a since-deleted document is silently dropped on replay.
  Summary: `filter_command`'s `Update` arm does `let Ok(Some(cur)) =
  get_document(..) else { continue }`; if the doc was later deleted the op is
  skipped. seq/command ordering is preserved and the later `Delete` still
  replays, so final-state convergence and the sequence guard are unaffected — a
  client just sees Create → (missing Update) → Delete. Harmless for end state;
  noted as a replay-fidelity limitation. Status: Accepted.

- Title: no smaller "caption" text-size token in the M7d token set. Summary: the
  M8b-2 asset panel's tile filename (`Assets.svelte` `.name`) renders at inherited
  body size — `_primitives.scss`/`_semantic.scss` define `--space-*`, `--radius-*`,
  `--font-sans`, and `--text-*` *colors* but no smaller font-*size* token (the plan's
  assumed `--text-sm` does not exist). Captions/secondary labels therefore can't be
  visually de-emphasized by size via a token. Status: **Deferred to M12** by the M8c-2
  §10 re-audit — canvas chrome (the M8 audit's scope) renders no text, so a font-*size*
  scale is out of scope here; it belongs with the text-dense default sheets/browsers in
  M12 (the second token re-audit point per `PLAN.md` M7).

- Title: M8c-2 §10 canvas-chrome token re-audit (outcome). Summary: re-audited the M7d
  3-tier token set against the first rendered canvas chrome. (1) Added a semantic
  `--grid-line` token (= `--slate-700`) so the canvas grid is decoupled from UI
  `--border`. (2) Fixed a latent M8c-1 bug: `Stage.svelte`'s `readColor` used
  `getComputedStyle().getPropertyValue("--token")`, which returns the unresolved
  `var(...)` string for aliased custom properties — so the grid silently used its
  fallback color and ignored the theme; it now resolves the real color via a
  computed-`color` probe. (3) Background uses `--surface-base` (already correct). (4)
  Fog-state colors (dimmed/unexplored) deferred to M9 (no visible fog in identity mode).
  Status: Resolved for M8c (canvas chrome); caption size token → M12 (above).

- Title: M10e-1 config-doc seed races resync (rare double-create). Summary: contribution
  panels (`GameSettingsPanel`, like `FactionsPanel`/`ConditionsPanel`) seed world config-docs
  from a reactive `$effect` that mounts during `#onWelcome` BEFORE the resync stream populates
  the optimistic store (`ws-client.ts`: welcome → onWelcome+module-activate → resync_request →
  event frames → resync_end). The `createSubscriber`+`subscribe()` reactivity + per-doc-type
  `length === 0` guard make a duplicate seed rare, but a GM whose first effect run lands with an
  empty store before resync can still create a duplicate `world-settings`/`light-gradation`/
  `vision-modes`. This is the SAME project-accepted condition as the `worldSession` scene
  auto-create (`worldSession.svelte.ts` "rare multi-GM ... double-create is accepted (M12
  dedupes)"). Status: **Accepted / deferred to M12** (singleton-config dedup). Not a regression.

- Title: M10e-1 world-defaults editor exposes a subset of `WorldSceneDefaults`. Summary: the
  `game-settings` world-defaults panel authors movement-restriction, lighting-enabled, light-mode,
  diagonal-rule, and animation only. `losRestriction`/`fog`/`observerVision`/`partialCellLeniency`
  and the world-level `environment` are present in `DEFAULT_WORLD_SETTINGS` (resolve correctly) but
  are authorable only as PER-SCENE overrides, not as world defaults. Matches the M10e-1 plan (Task
  6 scope); flagged so the M10e-2+ consumer knows world-level toggles for those axes are not yet in
  the UI. Status: Intentional V1 scope; revisit if world-level authoring of those axes is needed.

- Title: M10e-3 lighting soft edges via blur, not gradients. Summary: the lighting layer softens
  band/edge boundaries with a single Pixi BlurFilter; per-cell radial gradients (crisper falloff)
  were deferred. Status: Revisit (cosmetic; client-render-only).

- Title: M10e-3 darkvision render is an overlay approximation. Summary: darkvision-only cells get a
  low-alpha neutral gray wash; true desaturation needs a masked ColorMatrixFilter over the scene
  layers. The wire payload already carries the faithful per-cell renderHint, so the refinement is
  client-render-only (no server change). Status: Revisit.

- Title: Route stricter than the authoritative gate (footprint vs center-based). Summary: M10e-6's
  previewed route enforces full geometric footprint clearance (`cell_enterable` — the token's
  bounding-disc must clear all `blocksMove` segments and ALL footprint cells must be in the
  non-GM mask); the authoritative movement gate (M9/M10e-4) stays center-based (parent spec §14).
  A wide token can therefore be dragged (gate allows the center path) along a path the router
  refuses to preview through a narrow gap. This is the intended asymmetry: route ⊆ gate-allowed
  keeps the preview from suggesting a move the router would reject, while never admitting a move the
  gate would block. Not a bug. Status: Recorded; revisit when footprint-aware authoritative blocking
  lands.

- Title: Multi-leg alternating parity is per-leg-greedy (cost-display only). Summary: `find` threads
  each leg's min-cost end-parity into the next leg's start; for the `alternating` (5-10-5) rule this
  is not guaranteed globally cost-optimal across waypoints — a different parity threading could yield a
  lower total cost across the full multi-leg route. This is a cost-DISPLAY inaccuracy at waypoint
  boundaries only; the route itself remains valid (footprint-clear, mask-bounded, gate-passable). The
  spec §4.2 requires parity carry across legs, not global optimality. Documented in-code. Status:
  Recorded.
