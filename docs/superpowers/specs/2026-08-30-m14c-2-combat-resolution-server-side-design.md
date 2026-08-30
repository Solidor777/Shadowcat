# M14c-2 · Combat Resolution Server-Side — Design

**Status:** Approved (brainstorm 2026-08-30). Second of the six M14c sub-projects
([M14c-1 design](2026-08-30-m14c-1-server-formula-engine-design.md) §1). Amends the
[M14b design](2026-08-28-m14b-combat-clock-design.md) §4.2–4.3, §5.1 and §7.3, and completes the
amendment chain the M14c-1 spec opened at [M14 design](2026-08-28-m14-combat-tracker-design.md)
D4 — where those describe client-side formula resolution or `None`-skip behaviour, this spec wins.

## 0. What this fixes

Group 1 of the M14c-1 audit (its Appendix A): combat formula resolution designed client-side on
the overturned reading of invariant 6. Today, with the evaluator merged and unconsumed:

- `transition::recover` applies only `Formula::Number`; a `Formula::Text` recovery applies
  nothing.
- `ResourceBinding::Mirror` is dead — "the client keeps the combatant's numbers synced".
- `ResourceBinding::Tracked` is never seeded; the client wrote `CombatantResource.current/max`.
- `Duration.remaining` and `EffectLifecycle.resolved` are client-written `Option`s;
  `combat::effects::tick`/`expire_by_policy` skip any effect where either is `None`.
- The `combat` module's own doc invariant still reads "nothing here evaluates a `Formula::Text`".
- Route previews ignore the movement budget entirely; only `execute_move` enforces it.
- Resolved resource numbers are broadcast to every reader of the combatant document, including
  numbers derived from `gm_only` actor leaves.

The server evaluates all of it after this sub-project; the client evaluates only for display
(preview over the redacted documents it holds, deterministic by the shared conformance corpus).

## 1. Decisions

| # | Decision |
|---|---|
| C1 | **Formula host = token-embedded actor copy when present, else the linked actor.** One shared helper on the snapshot (`Combatant::formula_host`) resolves the document `formula::SystemLeafResolver` reads — the same join `SceneEcs::combatant_for_token` and `combat::effects::collect_effects` already perform, stated once. Every combat evaluation site takes the host from this helper; none re-derives the join. |
| C2 | **Evaluate on use; one stored home per value.** A value derivable from a formula over the host is never also stored. DELETED from the shapes: `EffectLifecycle.resolved`, `ResolvedLifecycle`, `CombatantResource.max`. The server computes lifecycle booleans and resource ceilings at each boundary/gate/egress use; nothing chases mid-combat actor edits to refresh a copy. |
| C3 | **Lazy-full semantics replace join-time seeding.** An absent `Tracked` entry in `CombatantEngine.resources` means untouched: `current = eval(max)`. The first spend, recovery that changes the value, or `CombatResource` `Set`/`Delta` writes the entry. `Duration.remaining: None` means not yet ticked = full: the first matching boundary writes `eval(amount) − 1` (expiry at `0` unchanged). No seeding writes, no mid-combat-joiner special case; history captures absent as absent. |
| C4 | **`Mirror` is pure derivation:** `current = max = eval(value)`. `CombatResource` on a Mirror-bound resource is refused (`CombatError`) — the number lives on the actor and is changed by writing the actor document through the normal path. A Mirror-bound `CombatEngine.movement.resource` is `MoveReject::BudgetUnresolvable`: a spend cannot decrement a derived value and the server never writes the `system` band, so the gate fails closed. |
| C5 | **Evaluation errors surface; they never brick the clock.** A `FormulaError` inside a transition skips that one write, the transition proceeds, and the failure surfaces as a GM-only System chat notice (existing notice machinery; deduplicated per transition). At the movement gate and the route-preview clamp it is the existing generic `BudgetUnresolvable`/`PathError`; on `CombatResource` it is a `CombatError` rejection. |
| C6 | **Resolved scalars default to trusted-only egress** (the whole-move-scalar rule). At combatant `Create`, ingress inserts `property_overrides["/engine/resources"] = owner_or_gm` unless the incoming document carries an explicit entry for that exact pointer. Server-enforced default riding the existing engine-band redaction (`REDACTABLE_BANDS` includes `engine`); GM opt-out is an explicit `all` entry. Derived, never-stored values need no rule: a client evaluates its preview over the redacted documents it holds, so a hidden leaf is `unknown-ref` for it. |
| C7 | **`Hard` route previews clamp inside `Pathfind` through the executor's own budget resolution.** The `BudgetGate`→`ResolvedBudget` resolution is extracted into a shared function both `Room::execute_move` and `handle_pathfind` call. Preview semantics mirror execution symbol-for-symbol; `PathResult` gains `truncated: bool`. |
| C8 | **Duration/ceiling numeric rule:** `remaining` seeds from `floor(eval(amount))`; a result `< 1` or non-finite is an evaluation error (C5), mirroring the authored `Formula::Number >= 1` ingress rule. Resource values stay `f64` and clamp to `[0, eval(max)]` as today. |

## 2. Evaluation sites

One internal helper family in `combat` (consuming `crate::formula::evaluate` +
`SystemLeafResolver` over C1's host):

- `resolved_resource(combatant, key) -> Result<ResolvedNums, FormulaError>` — Mirror ⇒
  `current = max = eval(value)`; Tracked ⇒ `max = eval(max)`, `current` = stored entry else `max`
  (C3). Consumed by `transition::recover` (apply + clamp), `transition::resource` (clamp;
  Mirror refused per C4), the movement gate's budget resolution (C7 shares it), and nothing else.
- `transition::recover` evaluates `Formula::Text` phase amounts; a zero-delta result still emits
  no write (unchanged rule).
- `combat::effects::tick`/`expire_by_policy` evaluate the lifecycle chain per boundary — the
  effect's authored `EffectLifecycle` formula, else the combat's snapshotted
  `CombatEngine.effect_lifecycle` default — truthy = non-zero. The `None`-skip branches are
  deleted. `tick`'s first matching boundary materializes `remaining` per C3/C8.
- The `combat` module doc invariant is rewritten: the server evaluates the engine's formula
  grammar over the combatant's host document; transitions never skip a value for being text.

An effect whose host document is gone is still skipped (unchanged — that is absence of the host,
not of resolution).

## 3. Egress (C6)

- Ingress: the combatant `Create` arm (beside the existing engine normalization at the
  persistence chokepoint) inserts the `/engine/resources` → `owner_or_gm` override when the map
  has no entry for that pointer. Updates never touch it (a GM's later explicit edit, including
  setting `all`, is an ordinary permissions write).
- `validate_property_overrides` already admits engine-band pointers; egress already strips
  per-recipient via `Access::can_see`. No new redaction machinery.
- Effect documents keep their host document's visibility (already the model): an effect's
  `remaining` is readable exactly where the effect itself is.

## 4. Route-preview clamp (C7)

- Extraction: the budget resolution currently inline in `Room::execute_move`
  (`SceneEcs::active_combat_for_scene` + `SceneEcs::combatant_for_token` + entry/`per_cell`
  resolution + `enforced` off the resolved `Access`) moves to one shared function; both callers
  consume it. `execute_move` behaviour is unchanged.
- `handle_pathfind`: when the request names a token (the only case with a combatant identity —
  hypothetical-footprint previews are never clamped), resolve the gate under the same read-guard
  pattern. Non-GM + active combat + combatant + `enforced` + `Enforcement::Hard`:
  - not the turn owner ⇒ the same generic `PathError` wording every other refusal uses;
  - else the returned route truncates at the last step whose cumulative unified cost
    (`GridShape::neighbors_with_cost` pricing, unchanged) fits the budget, and
    `PathResult.truncated = true`.
  `Warn`/`None`, GM, non-combatant tokens, and `enforced: false` (caller cannot read the
  combatant) are untouched and never disclose anything — mirroring the executor's disclosure
  discipline exactly.
- Parity test: preview clamp point == execution truncation point through the shared resolution
  for the same route/budget; sabotage-verified once each way (mutate either side ⇒ fail;
  restore ⇒ empty diff) and recorded in the plan, not kept.

## 5. Shape and client ripples

- Rust: `CombatantResource` loses `max` (struct stays, one field, so the doc comment keeps its
  home); `EffectLifecycle` loses `resolved`; `ResolvedLifecycle` deleted; `Duration.remaining`
  doc semantics become "server-written; `None` = full". `CombatantEngine::validate` and
  `CombatHistoryEngine::validate` adjust to the removed fields.
- ts-rs regeneration + client Zod mirrors + builders (`buildCombatantDoc`, `buildEffectDoc`)
  updated; every client helper that wrote resolved numbers back is deleted. Client display reads
  evaluate through `@shadowcat/formula` over the store's documents where a number is needed
  before M14c-6/M14d wire real UI.
- `MoveReject::BudgetUnresolvable` gains the Mirror-bound-resource case (C4) with the same
  generic wire wording.

## 6. Testing

- Invert the three interim tests (`text_recoveries_apply_nothing_server_side`, the
  `…skips_unresolved` pair) into evaluation assertions; fixtures gain real `system` bands the
  formulas read.
- Lazy-full: an absent Tracked entry gates and recovers as full; the first spend/tick
  materializes the entry; rewind restores materialized and absent states faithfully.
- Mirror: gate and display derivation; `CombatResource` refusal; Mirror movement resource ⇒
  `BudgetUnresolvable`.
- Evaluation errors: transition proceeds, the affected write is skipped, one GM-only notice per
  transition; gate/preview/`CombatResource` refusal paths.
- Egress: combatant Create stamps the default override; a non-owner recipient receives the
  document without `/engine/resources`; an explicit `all` entry at Create is respected; owner and
  GM see values.
- Clamp parity (§4) plus the truncation flag; not-turn-owner preview refusal wording equals the
  generic wording.
- The pure-transition harness (`validate_persisted`) continues to run real ingress gates over
  every stored document, now with populated `system` bands.

## 7. Security

- All evaluation inputs are documents the server holds; the M14c-1 caps bound every evaluation.
  Evaluation runs over unredacted documents by design — C6 is what keeps the derived outputs
  inside the owner/GM tier by default.
- No new disclosure surface at the preview clamp: refusals reuse the generic wording, clamping
  applies only to callers who can already read the combatant, and `truncated` reaches only the
  requester of their own preview.
- The server still never writes the `system` band (C4's Mirror rule is the enforcement corner of
  that).

## 8. Docs & skills

- `combat` module + affected function doc comments rewritten to the evaluated model (present
  tense, no history narration).
- `shadowcat-codebase-combat`: the "consumer wiring lands in M14c-2" interim invariant becomes
  the evaluated model; `shadowcat-codebase-formula` gains the combat consumer pointer;
  `shadowcat-codebase-scene-rendering` notes the shared budget resolution + preview clamp. All
  through the reviewed skill-update gate.
- M14b spec §4.2–4.3/§5.1/§7.3 gain "*Amended (M14c-2)*" pointers; PLAN.md marks M14c-2 done at
  completion; HISTORY.md entry at merge.
