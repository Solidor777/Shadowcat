# M14b · Combat Clock — Design

**Status:** Approved (brainstorm 2026-08-28). Checkpoint spec under the cross-cutting
[M14 design](2026-08-28-m14-combat-tracker-design.md); document shapes it does not restate live
there (§3). Where this spec and the M14 spec disagree, this spec wins and the M14 spec carries a
pointer at the amended decision.

## 1. Scope

Delivers the server-owned combat clock on top of the M14a document layer: the system-defaults
settings layer, the effect lifecycle model, the turn-history/rewind model, the combat intents and
their pure transition engine, the per-turn movement-budget gate, and the D15 cost unification.
Closes the three `docs/TODO.md` entries blocked on these (two movement-cost, one
one-active-per-scene batch).

Excludes (M14c/d): `AppContext.combat`, `CoreHooks` emission, `resolveResources` (the client half
of every `Formula::Text`), route-preview budget UX, the tracker module and settings editors. Every
client-side write this spec names (resolved numbers, system-defaults upsert) ships here only as
the `@shadowcat/core` builders and document contracts the M14c seams will call; no UI.

## 2. Decisions

| # | Decision |
|---|---|
| B1 | **A settings default is a layer, not a seed.** A new `system-defaults` singleton document holds the active system's declared defaults for EVERY world setting; resolution is engine fallback → system-defaults → world → scene everywhere a world setting is folded. Seed-once (`seedXIfAbsent`) remains only for registries, which are data, not defaults. |
| B2 | **A combat is `active` or not.** No created/paused/ended states. `CombatStart` activates (pausing any other active combat on the scene in the same command; a combat with `turn == None` initializes, one with `turn: Some` continues), `CombatPause` deactivates and nothing else runs, `CombatEnd` runs end-of-combat effect cleanup and DELETES the combat (cascade removes its children). |
| B3 | **Effects are hosted by the actor and clocked by the combatant.** The server expires an effect wherever it physically lives (token-embedded actor copy, linked actor, item-embedded with `transfer`); the combatant (`Duration.anchor`) owns its clock. Actor-side cleanup is embedding: an actor/token deletion takes its effects with it. |
| B4 | **Effect cleanup is policy, layered like every setting:** a master switch (`effect_cleanup`, default on) and four per-effect policy formulas with system/world/scene defaults. Duration expiry by counting is the effect's own clock and is NOT gated by the master switch. |
| B5 | *Amended (M14c-1):* the server evaluates formulas; see the [M14c-1 spec](2026-08-30-m14c-1-server-formula-engine-design.md). Original text: **`Duration.amount` and every lifecycle policy are `Formula`s** resolved by the client (D4 split); the server reads only the resolved numbers/flags it is handed and skips an effect that has none — a server-side fallback would be a second resolver. |
| B6 | **Turn history is a snapshot per turn; rewind restores.** Every turn boundary captures the engine bands of all combatants and anchored effects; `CombatRewind` writes them back (mid-turn spends, GM tweaks and expiries all return to the boundary state). Deltas are derived, never stored. Replaces D16's "rewind never un-expires" with exactness. |
| B7 | **Rewind/fast-forward restore are settings** (`rewind_restore` default `true`, `forward_restore` default `false`, chain-resolved). Future records are retained past a rewind only when `forward_restore` is on; fast-forward restores one only when the live state equals the current record, otherwise the future is discarded and the transition runs. |
| B8 | **History is a GM-only child document** (`combat-history`, `permissions.default: none`), dropped whole at egress by the M14a READ gate, so hidden combatants never leak through it. Bounded to the newest 200 turns. |
| B9 | **Cost is one quantity.** `execute_move` and the router share `step_cost`; `los_smooth` recomputes exact chord cost. The movement gate consumes that number. |

## 3. System-defaults layer (B1)

### 3.1 Document

`system-defaults`: singleton engine doc type (`SINGLETON_DOC_TYPES`, deterministic id
`deterministicId(worldId, "system-defaults")`), `deny_unknown_fields`, ts-rs exported, validated at
`validate_engine_tree`. The registry grows 21 → 23 with this and `combat-history` (§6).

```
SystemDefaultsEngine {                       // every leaf optional; absent = fall through
  scene:       Option<SceneDefaultsOverlay>, // Option<> per field of WorldSceneDefaults
  pathfinding: Option<PathfindingOverlay>,
  animation:   Option<AnimationOverlay>,
  combat:      Option<CombatDefaults>,       // already fully optional (M14a)
}
```

`active_scene` is world state, not a setting, and has no overlay. Overlay structs are the
`Option`-lifted twins of the world structs; a field added to a world struct without its overlay
twin fails the mirror test (§3.4).

### 3.2 Resolution

One folder per family, each taking the system layer as its innermost non-engine layer:

- `SceneEcs::resolve_scene` — engine literal → `system.scene` → `world.scene` → scene `vision`/
  `lighting` overrides. The engine literals stay where they are; the system layer slots between
  them and the world read.
- `SceneEcs` pathfinding/animation readers (`resolved_animation_speed`, the diagonal-rule reader)
  — same insertion.
- `combat::resolve_combat_rules(system, world, scene)` — gains the third argument; `Some(None)`
  clears propagate through all four layers exactly as they do through two today.

The ECS holds the `system-defaults` doc beside `world_settings` and reads it through the same
`engine_as_cached` path. Client: `resolveSceneSettings` and the M14c combat resolver take the
same layer from the store; `DEFAULT_WORLD_SETTINGS` remains the engine literal.

### 3.3 Ownership

- "The active system" gets an identity: the module providing the singleton contract
  `SYSTEM_CONTRACT = "shadowcat.system"` (the registry already elects one winner per singleton
  contract; `ModuleRegistry.systemModule()` returns it). A module declares
  `Module.systemDefaults?: SystemDefaultsEngine` on the module object (beside `manifest`, so no
  manifest-schema change). On world join the GM's client (cooperative-trust write, as for
  registries) compares the declaration to the stored doc and dispatches one field Update per
  differing top-level section (real OCC pre-images), or a Create when absent. Switching systems
  therefore re-applies. No system ⇒ no doc ⇒ empty layer.
- Ingress: GM-only write (`core:create`/edit capability at the GM tier); a non-GM write is
  refused at `apply_intent` like any other unauthorized write.
- The GM never edits the doc directly. The game-settings panel shows, per setting, the effective
  value and its provenance (`engine | system | world | scene`) and offers **reset to system
  default**. A world leaf that is required on the wire (`WorldSceneDefaults`, `Pathfinding`,
  `AnimationSettings` fields) cannot be removed without failing ingress, so reset writes the
  system-resolved value into it; provenance reports `world` for such a leaf only when its stored
  value differs from the layer beneath. Optional leaves (`combat.*`) are removed
  (`remove: true`) so resolution falls through.

### 3.4 Tests

Precedence per family (engine < system < world < scene), `Some(None)` clear through four layers,
overlay/world struct field-set parity (a compile-time or reflective mirror test), re-application
on a changed declaration, GM-only ingress, provenance rendering.

## 4. Effect lifecycle (B3–B5)

### 4.1 Shapes (amend M14 §3.4)

```
EffectEngine { active: bool, transfer: bool, duration: Option<Duration>, lifecycle: Option<EffectLifecycle> }

Duration {
  amount: Formula,             // authored; opaque to the server
  remaining: Option<u32>,      // resolved counter the server moves; None = not on a clock yet
  unit: Rounds | Turns,        // Rounds = round wraps; Turns = the anchor's turn boundaries
  anchor: Option<Uuid>,        // combatant id; None = the combatant whose actor hosts the effect
  expires: TurnStart | TurnEnd | RoundStart | RoundEnd,
}

EffectLifecycle {              // authored; every field optional → chain (system → world → scene → engine)
  on_combat_end: Option<Formula>,   // truthy ⇒ expire at CombatEnd        engine fallback: expire
  on_turn_end:   Option<Formula>,   // truthy ⇒ expire at host's turn end   fallback: keep
  on_advance:    Option<Formula>,   // truthy ⇒ decrement `remaining`       fallback: decrement
  resolved: Option<ResolvedLifecycle>,       // { on_combat_end, on_turn_end, on_advance }: bool
}

CombatDefaults += effect_cleanup: Option<bool>,                       // fallback true
                  effect_lifecycle: Option<EffectLifecycleDefaults>,  // the three Formulas
                  rewind_restore: Option<bool>,                       // fallback true
                  forward_restore: Option<bool>                       // fallback false
CombatEngine   += effect_cleanup: bool, rewind_restore: bool, forward_restore: bool,
                  effect_lifecycle: EffectLifecycleDefaults                          // snapshot (D7)
```

`ClockStamp`/`started` is removed: the history (§6) records where every effect stood at every
boundary, so nothing needs to reconstruct a start point.

### 4.2 Resolution (client, D4)

*Amended (M14c-1):* the server evaluates formulas; this subsection describes the retired
client-resolution model. See the [M14c-1 spec](2026-08-30-m14c-1-server-formula-engine-design.md).

When an effect joins a clock (its host becomes a combatant of an active combat, or the effect is
created on such a host) the client evaluates `amount` → `remaining`, and the three policy
formulas through the chain → `resolved` (a `Text` formula evaluating non-zero ⇒ the "act"
branch). It re-resolves when the effect's formulas or the chain change. The server never reads
`amount` or the policy formulas; an effect with `remaining: None` or `resolved: None` is skipped
by every transition.

### 4.3 Server semantics

- **Boundary tick** (the anchor's boundary matching `expires`; `Rounds` = round wraps, `Turns` =
  the anchor's own turn boundaries): if `resolved.on_advance`, `remaining -= 1`; at `0`,
  `active = false`. The client derives `combat:effect-tick` / `combat:effect-expired` from the
  delta (M14c).
- **`on_turn_end`**: at the HOST combatant's turn end, `active = false` when
  `resolved.on_turn_end && combat.effect_cleanup`.
- **`on_combat_end`**: at `CombatEnd`, `active = false` for every effect anchored to any of the
  combat's combatants when `resolved.on_combat_end && combat.effect_cleanup`. `CombatPause`
  touches no effect.
- **Location** (B3): the transition loads the combatant's token and actor documents, walks
  `embedded.effect[*]` and `embedded.item[*].embedded.effect[*]` (the latter only where
  `transfer`), and writes index-addressed field Updates (`/embedded/effect/<i>/engine/...`) with
  OCC pre-images. An effect is identified by `(host doc id, embedded path)`; a host that no longer
  exists is skipped, never an error.

## 5. Combat intents and transitions (B2; amends M14 §5)

New `ClientMsg` variants, dispatched in `conn.rs` beside `MoveRequest`. Each `handle_*` loads the
combat, its `combatant` children, the `combat-history` child and every host document, runs a
**pure** transition (`src/server/src/combat/transition.rs`: `CombatSnapshot → Result<Vec<Operation>,
CombatError>`; no DB, no ECS) and commits the ops through `Room::commit_ops_locked` under
`publish_guard` as ONE command with `WriteOrigin::CombatTransition`: a server-authored origin
that skips `apply_intent`'s per-op capability gates (an owner's `CombatAdvance` writes other
combatants' recoveries and host-embedded effects) while every validation — scope, size, engine,
containment, singleton, one-active-per-scene, schema, OCC — still runs. No wire frame can select
an origin. A failure returns `ServerMsg::CombatError { request_id, message }` to the originator
only; the message never distinguishes "hidden" from "absent" or "not yours".

| Intent | Authz | Effect |
|---|---|---|
| `CombatStart { combat_id }` | GM | any other active combat on the scene: `active = false` (same command). Then: `turn == None` ⇒ resolve the 4-layer chain into the snapshot fields, `round = 1`, `turn = order[0]`, run `round_start` + the first `turn_start` phase (with the auto-resolve loop below), write the first history record; `turn: Some` ⇒ `active = true` only. |
| `CombatPause { combat_id }` | GM | `active = false`. |
| `CombatEnd { combat_id }` | GM | §4.3 `on_combat_end`; then `Delete` of the combat (cascade: combatants, history). |
| `CombatAdvance { combat_id }` | GM; the current combatant's owner under `OwnerMayEnd` | §5.1 |
| `CombatRewind { combat_id }` | GM | §6.2 |
| `CombatRoll { combat_id, channel, rolls: [{ combatant_id, notation }] }` | GM; owner for own combatant | each through `chat::rolls::execute_roll` (the sole untrusted-notation path; its caps and entropy apply unchanged) with the dice context resolved for `channel`; write `initiative`; rebuild `order`; one chat-message `Create` per roll on `channel` in the same command carrying a `RollEmbed` (hidden combatant ⇒ GM-only audience). |
| `CombatResource { combat_id, combatant_id, resource, op: Delta(f64) \| Set(f64) }` | GM; owner | clamp to `[0, max]`; non-finite refused. |
| `CombatSort { combat_id }` | GM | `order` ← `initiative desc, tiebreak desc, existing index`. |

`order` stays the single authority on sequence; `CombatRoll`/`CombatSort` rebuild it, a GM
reorder is a plain field Update. `CombatEngine.validate` (order unique, `turn ∈ order`) runs on
every write as today.

### 5.1 `CombatAdvance`

With future history records and `forward_restore` on, §6.2 fast-forward applies first. Otherwise
(future records, if any, are truncated — §6.2):

1. Current combatant: `turn_end` numeric recoveries; `TurnEnd` ticks for effects anchored to it;
   `on_turn_end` cleanup (§4.3).
2. Next = `order[(i+1) % len]`. On wrap: `round += 1`; `round_end` then `round_start` numeric
   recoveries for every combatant; `RoundEnd`/`RoundStart` ticks.
3. Next: `turn_start` numeric recovery; `TurnStart` ticks.
4. If next is an `Event`: `Create` its chat message if set (GM-only audience when hidden);
   `lifespan -= 1` and `Delete` the combatant at `0` (and remove it from `order`); run step 1 for
   it; go to 2.
5. Else if next is hidden and `turn_control = OwnerMayEnd`: run step 1 for it; go to 2.
6. Else `turn = next`; append the history record (§6.1).

Loop guard: at most `order.len()` iterations; an all-event/all-hidden order terminates with `turn`
on the last visited entry and the round advanced. A `Formula::Number` recovery is applied and
clamped to `max`; a `Formula::Text` recovery applies nothing (client half, M14c). A combatant
whose token/actor is gone stays an unresolved row and never panics a step.

### 5.2 One-active-per-scene batch (closes the `TODO.md` entry)

`CombatStart`'s swap is a deactivate-then-activate on one scene in one command, exactly the batch
`apply_intent` over-rejects today. Fix: Phase 1's `Create` and `Update` arms both consult and
update the batch-local `claimed_active_scenes`/`released_active_scenes` sets before the database
check, so a scene released earlier in the same batch may be claimed later in it. Two activations
of different combats on one scene in one batch still reject. Test: swap batch in both op orders
passes; double-activate rejects; the existing single-op cases are unchanged.

## 6. Turn history and rewind (B6–B8)

### 6.1 Document

`combat-history`: child document (`parent_id = combat.id`, exactly one per combat, created by the
first `CombatStart`), `permissions.default: none` (GM-only; `owner` never listed), engine doc type
joining the registry. Deleted with its combat.

```
CombatHistoryEngine {
  records: Vec<TurnRecord>,        // oldest first; at most MAX_TURN_HISTORY = 200
  cursor: u32,                     // index of the record describing the CURRENT turn
}
TurnRecord {
  round: u32, turn: Uuid,
  combatants: Vec<{ id, parent_id, permissions, owner, engine: CombatantEngine, system }>,  // whole child docs
  effects:    Vec<{ host: Uuid, path: String, engine: EffectEngine }>,                  // anchored effects
}
```

A record is captured by every transition that lands on a new `turn` (start, each advance, and the
auto-resolved intermediate steps of §5.1 steps 4–5 — so a rewind can land on an event's or a
hidden combatant's turn and replay it). When `records.len()` would exceed the cap, the oldest
record drops; `cursor` shifts with it.

### 6.2 Rewind and fast-forward

- **`CombatRewind`**: refused when `cursor == 0` (the round-1 floor is the first record). Else
  `cursor -= 1`; set `round`/`turn` from the record. When `rewind_restore`: for every captured
  combatant, an `Update` of its engine (and `system`) band with the CURRENT value as pre-image, or
  a `Create` when it no longer exists (a deleted event); for every captured effect, the same on
  its host path (skipped when the host is gone). When `rewind_restore` is off, only `round`/`turn`
  move. Chat posts are never retracted; no recovery runs. When `forward_restore` is off (the
  default) the rewind also truncates `records[cursor+1..]` in the same command — future state is
  kept only when something can restore it, so `cursor == records.len() - 1` is an invariant in
  that mode.
- **Fast-forward** (inside `CombatAdvance`): applies only when `cursor < records.len() - 1` AND
  `forward_restore` AND the live state of every combatant and anchored effect equals
  `records[cursor]` (an exact comparison of the captured bands; a rewind with `rewind_restore` off
  never satisfies it). Then `cursor += 1` and `records[cursor]` is restored as above — no
  transition, no recoveries, no chat posts. When the comparison fails or the setting is off,
  `records[cursor+1..]` are truncated and §5.1 runs.
- Discarding on modification is detected at advance time by that comparison, not by a hook on
  every combatant write. The M14d tracker hides the fast-forward affordance whenever its own view
  sees live state ≠ current record.

### 6.3 Tests

Record captured per boundary incl. intermediate auto-resolved steps; rewind restores mid-turn
spends, GM edits, expired effects, deleted events; floor at record 0; `rewind_restore` off moves
only the clock; `forward_restore` off truncates the future on rewind; fast-forward restores when
equal and truncates when not; cap drops oldest and
shifts `cursor`; history absent from a player's Welcome/broadcast/resync and present for the GM.

## 7. Movement gate and cost unification (B9; amends M14 §5 gate, §9)

### 7.1 Cost unification (first — the gate consumes it)

- `move_exec::execute_move` prices each cell transition through
  `GridShape::neighbors_with_cost(prev_cell, parity)` — the same call `pathfinding::find`'s
  arrest replay uses — threading the returned parity: `cost += step × terrain_multiplier(next_cell)`.
  (Not `step_cost` directly: an axial hex step has two non-zero components and would price as a
  diagonal; the trait keeps hex at 1.0.) On a `Continuous` scene a span is priced as its
  Euclidean length in cells × the entered cell's multiplier, which is what the continuous router
  reports. `MoveOutcome.cost` is then the router's number for the same route.
- `navmesh::los_smooth` recomputes each straightened chord's cost exactly: per-cell span
  integration (terrain-weighted Euclidean length within each crossed cell), replacing the
  pre-smoothing conservative value.
- Parity test per `DiagonalRule` × grid kind × movement model: route → execute → equal cost;
  mutate either side → the test fails (sabotage verified once, then removed).
- Both movement-cost `TODO.md` entries close.

### 7.2 Gate inputs

The ECS hydrates `combat` and `combatant` documents into a **combat index**: active combat per
scene, combatant by `token_id` and by `actor_id`. `Room::execute_move` resolves, under the same
scene read lock as every other gate input, `Option<BudgetGate { combatant_id, is_turn_owner,
current, per_cell: Option<f64>, interpretation, enforcement }>` — synchronous, no repo await in
the critical section. The token's combatant is found by `token_id`, else by the token's resolved
actor id.

### 7.3 Decision

Non-GM, active combat with `movement.resource = Some(r)`, token is a combatant:

- Not the turn owner: `Hard` ⇒ `MoveReject::NotYourTurn`; `Warn`/`None` ⇒ allowed.
- Budget cells: `PerCell ⇒ current / per_cell`; `Spaces ⇒ current`. Missing scene
  `grid.distance.perCell` under `PerCell`, or no `resources[r]` on the combatant ⇒
  `MoveReject::BudgetUnresolvable` — no synthesized default.
- `Hard` ⇒ `execute_move` takes `budget: Option<f64>` and stops at the last dense step whose
  cumulative unified cost ≤ budget (`truncated = true`).
- Both new rejects map to today's generic `"move rejected"` (no budget disclosed).

GM: never truncated, never `NotYourTurn`. **Every** executed move by a combatant (GM, `Warn`,
`None` included) appends `resources[r].current -= cost` (floor 0) to the same command as the
position write. §6 rewind restores it.

## 8. Security

All intents authz-checked server-side per the table; hidden combatants unreadable, history
GM-only, roll posts whispered, one indistinguishable error wording; `Formula::Text` opaque
server-side; `system-defaults` GM-write only; no budget value leaves the owner/GM tier.

## 9. Phases

1. System-defaults layer (§3): doc type, four-layer resolvers server + client, manifest field +
   upsert builder, game-settings provenance UI.
2. Shapes (§4.1, §6.1): effect lifecycle, `CombatDefaults`/`CombatEngine` additions,
   `combat-history` type, ts-rs/Zod, builders; the `apply_intent` batch fix (§5.2).
3. Cost unification (§7.1) + `TODO.md` closures.
4. Intents + pure transition engine + history/rewind (§5, §6).
5. Movement gate + ECS combat index (§7.2–7.3).
6. Docs sync (`PLAN.md`, `HISTORY.md`, `TODO.md`, M14 spec pointers), `shadowcat-codebase-combat`
   skill update through the reviewed skill-update gate.

Buddy-check pre-authorized for phases 4–5 (transition engine, rewind, gate) and for §5.2.
