# M14 · Combat Tracker — Design

**Status:** Approved (brainstorm 2026-08-28). Cross-cutting spec; four checkpoints (M14a–d),
each with its own plan cycle.

## 1. Goals & placement

Phase-2 entry milestone. Delivers the engine-owned combat clock every later Phase-2 feature keys
off (M17 turn-owner vision work, M18 condition/damage triggers), closes the two `docs/TODO.md`
entries blocked on a per-turn movement budget, and undoes the M13 mistake of leaving `effect` a
system-only document.

Depends on: M11 dice (server-only roll engine + `chat::rolls` caps/entropy), the M10 movement
executor (`move_exec::execute_move`, `Room::execute_move`), the M13 `effect` document
(Nightfox-side today), the M12 panel system, the M13 formula library + system resolver seam.

Excludes: automation of attacks/damage resolution (system-owned — the engine ships hooks and an
event kind, never damage); audio/VFX cues (Phase 3); Nightfox's own migration onto the engine
`effect` band and resource registry (a follow-up in the Nightfox repo, out of this milestone);
movement-type exemptions (M17).

## 2. Decisions locked

| # | Decision |
|---|---|
| D1 | **`effect` is an engine doc type.** It gains a typed `engine` band (`active`, `transfer`, optional clock-bound `duration`). Modifiers stay in `system`. "Effect should never have been Nightfox-only" (user, 2026-08-28). |
| D2 | **Generic turn-resource system, nothing hooked up by default.** Named resources are defined as data in a `resource-registry` config doc; the engine ships an EMPTY registry. Movement is one such resource, selected by configuration, not built in. Nightfox (out of scope) will define `movement` + `actions`; tests use an equivalent fixture system. |
| D3 | **Two resource bindings.** `Mirror` — the combatant's value continuously equals a formula over its actor (e.g. HP ≡ actor health); `Tracked` — the value is combat state seeded from a formula to `max` and replenished per turn/round boundary (e.g. movement). |
| D4 | **Formulas are opaque to the server** (ARCHITECTURE invariant 6 kept). Resource definitions store formulas as strings; the CLIENT (active system's resolver via `@shadowcat/formula`) evaluates them and writes NUMBERS into the combatant. The server only ever reads and arithmetically updates numbers. |
| D5 | **Server-owned clock.** Turn/round transitions, recovery of numeric amounts, effect expiry, hidden auto-skip, event firing and the movement budget gate all execute server-side, atomically, inside ONE command per intent. Client hooks are derived from applied command deltas so every client observes identical events. |
| D6 | **Movement interpretation modes**: `PerCell` (budget ÷ scene `grid.distance.perCell` ⇒ cells: 30 ft / 5 ft = 6 cells) and `Spaces` (budget IS cells). **Enforcement modes**: `None` (engine default), `Warn` (route preview shows the overage during drag), `Hard` (the walk truncates at the last affordable step). **GMs are never truncated** (as they are never wall-gated); their moves still decrement. |
| D7 | **Override chain system → world → scene** for movement resource, interpretation, enforcement and turn control: the system module seeds the world default alongside the registry; `WorldSettingsEngine.combat` and `SceneEngine.combat` override field-by-field; the combat doc SNAPSHOTS the resolved chain at start (a mid-fight settings edit never silently rewrites the rules of a running combat). |
| D8 | **Combat = world doc bound to a scene; many per world, at most one ACTIVE per scene** (ship map + ground map run independently). **Combatants = embedded documents** in the combat doc (`embedded.combatant[]`), linked to a token and/or actor, carrying all per-combat state. |
| D9 | **Hidden combatants are stripped at egress** for non-GM recipients (redaction, not send-then-hide — stripping costs no UX here). Nothing observable betrays a hidden entry: no placeholder row, no count, no counter tick, no distinguishable error. |
| D10 | **Turn control** `OwnerMayEnd` (default): the GM may do anything; the owner of the current combatant may `CombatAdvance` (end own turn) only. `GmOnly`: only the GM advances. |
| D11 | **Hidden turns auto-resolve under `OwnerMayEnd`**: when the order reaches a hidden combatant its turn STARTS and immediately ENDS inside the same command — turn effects, recoveries and event actions still resolve — so no dead time reveals it. Under `GmOnly` hidden turns are held normally. |
| D12 | **Events are a combatant kind**, not a second list: a named entry in the initiative order with an optional lifespan in turns (default infinite) and a default chat-message action; may be hidden. Its turn always starts-and-ends in one command (no owner to act). System-defined event behaviour rides the `combat:turn-start` hook + the combatant's opaque `system` band. |
| D13 | **Initiative is server-rolled** via a `CombatRoll` intent using the existing `chat::rolls` cap/entropy layer, written atomically into the combat doc and posted to chat with a `RollEmbed` (hidden ⇒ GM-only whisper). Client composes notation exactly as the M13d roll wire does. |
| D14 | **The tracker is a module** (`src/modules/combat`) that talks only to `AppContext.combat` (`shadowcat.service:combat`) + `ctx.documents`; any system or third-party module may replace or extend it. |
| D15 | **Execution cost and preview cost become one quantity** (closes both blocked `TODO.md` entries): `execute_move` threads the diagonal rule + per-step parity through the SAME step-cost function the router uses; `los_smooth` recomputes exact per-span cost for straightened chords. Pinned by a parity test. |
| D16 | Effect expiry sets `active: false`; it never deletes. Rewind never un-expires. |

## 3. Documents

All new engine types join `is_engine_doc_type` / `normalize_engine` in `data/engine/`, are
`#[serde(deny_unknown_fields)]`, ts-rs exported, Zod-mirrored, and validated at the existing
`validate_engine_tree` chokepoint (which already recurses into `embedded`).

### 3.1 `combat` (world-level; `scene_id` bound)

```
CombatEngine {
  scene_id: Uuid,
  active: bool,                      // at most one active combat per scene (server-enforced at
                                     // the apply_intent Create/Update chokepoint, same mechanism
                                     // as SINGLETON_DOC_TYPES but keyed on scene_id)
  round: u32,                        // 0 = created, not started
  turn: Option<Uuid>,                // current combatant's embedded doc id
  turn_control: TurnControl,         // OwnerMayEnd | GmOnly (snapshot of the chain, D7)
  order: Vec<Uuid>,                  // resolved order; GM-reorderable by field Update
  movement: MovementRules,           // snapshot of the chain (D7)
}
MovementRules { resource: Option<String>, interpretation: PerCell | Spaces, enforcement: None | Warn | Hard }
```

Ordering: `order` is rebuilt by `CombatRoll` and by `CombatSort` (GM) as
`initiative desc, tiebreak desc, then existing index`; a GM's manual reorder is a plain field
Update on `order`. Adding a combatant appends. `order` is the single authority on sequence —
nothing re-derives it from `initiative` at read time (no forked decision).

### 3.2 `combatant` (embedded only: `combat.embedded.combatant[]`)

```
CombatantEngine {
  kind: Actor { token_id: Option<Uuid>, actor_id: Option<Uuid> }   // ≥1 required
      | Event { lifespan: Option<u32>, message: Option<String> },  // lifespan in turns; None = infinite
  initiative: Option<f64>,
  tiebreak: f64,                                    // system/GM-supplied secondary key (default 0)
  hidden: bool,
  resources: BTreeMap<String, CombatantResource>,   // keyed by resource-registry id
}
CombatantResource { current: f64, max: f64 }        // numbers only (D4)
```

`system` band on a combatant is the system's (event parameters, per-combat system state).
Token → actor resolution reuses `resolveTokenActor` (token's `engine.actor_id` first, embedded
copy fallback). A combatant whose token or actor is deleted stays in the order as an unresolved
row (the GM removes it); it never panics a transition.

### 3.3 `resource-registry` (singleton config doc; seeded by the system module, GM-editable)

```
ResourceRegistryEngine { resources: BTreeMap<String, Resource> }     // map, single-key updates
Resource {
  name: String, order: u32,
  binding: Mirror { value: Formula }
         | Tracked { max: Formula, recover: Recovery },
}
Recovery { turn_start: Formula, turn_end: Formula, round_start: Formula, round_end: Formula }  // each default "0"
Formula = Number(f64) | Text(String)     // Text = @shadowcat/formula source, OPAQUE to the server
```

Seeding: the same `seedXIfAbsent` + `deterministicId(worldId, "resource-registry")` pattern as
the condition registry. Engine default: no resources. A `Tracked` resource seeded "full" is
expressed as `max: <formula>` + `recover.turn_start: <the same formula>` — the Nightfox movement
shape (`max: "speed"`, `turn_start: "speed"`).

### 3.4 `effect` (engine band added; embedded under actors/items as today)

```
EffectEngine {
  active: bool,                 // default true
  transfer: bool,               // default false (item-embedded effect reaches the owning actor)
  duration: Option<Duration>,   // None = on-while-active (today's semantics)
}
Duration {
  amount: u32,
  unit: Rounds | Turns,          // Turns count the anchor combatant's turn boundaries; Rounds count round wraps
  anchor: Option<Uuid>,          // combatant id; None = the combatant whose actor hosts the effect
  expires: TurnStart | TurnEnd | RoundStart | RoundEnd,
  started: { round: u32, turn_index: u32 },   // written by the server when the effect is created
                                              // during an active combat, else by the client
}
```

### 3.5 Override chain (D7)

```
CombatDefaults { movement_resource?: Option<String>, interpretation?: Interpretation,
                 enforcement?: Enforcement, turn_control?: TurnControl }   // every field optional
WorldSettingsEngine.combat: Option<CombatDefaults>
SceneEngine.combat:         Option<CombatDefaults>
```
Engine fallback when nothing is set: no movement resource, `PerCell`, `None`, `OwnerMayEnd`.
`CombatStart` resolves scene ⊕ world ⊕ engine fallback into the combat doc's snapshot fields.

## 4. Formula evaluation split (D4)

- Server: stores `Formula::Text` verbatim under the existing size caps; never parses it. Applies
  `Formula::Number` recoveries itself inside transitions. For a `Formula::Text` recovery it applies
  nothing and relies on the client half.
- Client (`AppContext.combat.resolveResources(combatantId)`): resolves every registry formula
  through the active system's resolver against the combatant's actor (token-embedded copy or
  linked actor, via the existing `EffectiveActor` read-through) and writes `{current, max}`
  numbers with field Updates carrying real OCC pre-images. Runs on combat join, on any change to
  the actor/registry (mirror resources), and in response to a boundary hook whose recovery is
  `Text` (the client computes the amount, adds it, clamps to `max`, writes `current`). The GM's
  client is authoritative for these writes (cooperative-play trust model, as for all system
  data); a non-GM owner may write only its own combatant's resources.
- Static numbers need no resolver: the tracker exposes editable `current`/`max` fields.

## 5. Server intents & transitions (D5)

New `ClientMsg` variants, dispatched in `conn.rs` beside `MoveRequest` (private `handle_*`
returning `Option<ServerMsg>` — `Some` = error frame to the originator only), each applied via
`Room::publish` as ONE `Command`:

| Intent | Authz | Effect |
|---|---|---|
| `CombatStart { combat_id }` | GM | resolves D7 snapshot, `round = 1`, `turn = order[0]`, runs the first combatant's `round_start` + `turn_start` phases (incl. D11/D12 auto-resolution), marks `active`, refuses if another combat is active on the scene |
| `CombatEnd { combat_id }` | GM | `active = false`, `turn = None`; effects untouched |
| `CombatAdvance { combat_id }` | GM; or owner of the current combatant when `turn_control = OwnerMayEnd` | the transition below |
| `CombatRewind { combat_id }` | GM | previous entry (wrap ⇒ `round -= 1`, floor 1); no recovery, no un-expiry (D16) |
| `CombatRoll { combat_id, rolls: [{ combatant_id, notation }] }` | GM; owner for their own combatant | executes each through the shared `chat::rolls` cap/entropy path, writes `initiative`, rebuilds `order`, posts one chat message per roll (`RollEmbed`; hidden ⇒ GM-only whisper) |
| `CombatResource { combat_id, combatant_id, resource, op: Delta(f64) \| Set(f64) }` | GM; combatant owner | server clamps to `[0, max]` |
| `CombatSort { combat_id }` | GM | rebuilds `order` from `initiative desc, tiebreak desc, existing index` without rolling |

**`CombatAdvance` transition** (all in one command; hooks derive from its deltas):
1. Current combatant: `turn_end` numeric recovery; effects anchored to it whose `expires =
   TurnEnd` reach zero ⇒ `active = false`.
2. Next = `order[(i+1) % len]`; on wrap: `round += 1`, `round_end` then `round_start` numeric
   recoveries for every combatant, `RoundEnd`/`RoundStart` expiries.
3. Next combatant: `turn_start` numeric recovery, `TurnStart` expiries.
4. If next is an `Event`: post `message` if set (GM-only when hidden), decrement `lifespan`
   (remove the entry at 0), then run step 1 for it and go to step 2.
5. Else if next is `hidden` and `turn_control = OwnerMayEnd`: run step 1 for it and go to step 2
   (D11).
6. Else `turn = next`.
A loop guard bounds the iteration at `order.len()` steps (an all-hidden/all-event order
terminates with `turn` on the last visited entry and a round advanced).

**Movement gate** (`Room::execute_move`, before `move_exec::execute_move`): if the token's scene
has an active combat with `movement.resource = Some(r)` and the token (or its actor) is a
combatant `c`:
- not the turn owner (and not GM): under `Hard` ⇒ `MoveReject::NotYourTurn` (rejected like any
  other move error — no geometry, no budget disclosed); under `Warn`/`None` ⇒ allowed.
- budget cells = `PerCell ? c.resources[r].current / grid.distance.perCell : current`. An
  absent scene `grid.distance` under `PerCell`, or a combatant lacking the resource entry, refuses
  with `MoveReject::BudgetUnresolvable` (sibling of `SceneUnknown`) — no synthesized default,
  per the fail-open-defaults rule.
- `Hard` and not GM ⇒ pass `budget` into `execute_move` as a truncation limit; the walk stops at
  the last step whose cumulative cost ≤ budget (`truncated = true`). Cost is the unified D15
  quantity.
- Every executed move (GM included) decrements `current` by the cost walked, floored at 0, in the
  same command as the position write.

**Redaction (D9)**: hidden combatants are removed from the embedded list per recipient at every
egress (Welcome/snapshot, broadcast, resync — the same per-recipient filter documents already
pass through). While a hidden combatant holds `turn` (only possible under `GmOnly`), non-GM
recipients receive `turn: null`. `CombatAdvance` from a non-GM during a hidden turn returns the
same error as "not your turn".

## 6. Client seams (`@shadowcat/core` / ui-kit)

- ts-rs → Zod mirrors for every new type and frame (drift guard).
- `AppContext.combat` (`shadowcat.service:combat`): `activeFor(sceneId)`, `start/end/advance/
  rewind(id)`, `roll(id, rolls)`, `modifyResource(id, combatantId, resource, op)`,
  `resolveResources(combatantId)`, `remainingMovement(tokenId) → { current, cells } | null`,
  `addCombatant/addEvent/remove/setHidden/reorder` (thin wrappers over field Updates +
  `manage_embedded`).
- First-party hooks, the first entries in `CoreHooks`: `combat:start`, `combat:end`,
  `combat:round-start`, `combat:round-end`, `combat:turn-start`, `combat:turn-end`,
  `combat:effect-tick`, `combat:effect-expired` — kind `info`, payload
  `{ combatId, round, combatantId?, kind?: "actor" | "event", effectId? }`. Emitted by the core
  store from applied-command deltas (before/after `round`, `turn`, effect `active`), identically
  on every client; hidden entries never appear in a non-GM's payloads because they never reach
  that client.
- Route preview (scene-tools measure/route mode) reads `remainingMovement`: `Warn` renders the
  overage label during drag; `Hard` clamps the previewed route at the affordable prefix so
  preview == outcome.

## 7. Tracker module (`src/modules/combat`, D14)

Panel contribution (`PANEL_CONTRACT`, `order` after conditions, default placement docked right).
Rows: token art/name (redacted name ⇒ display name fallback), initiative, resources (mirror
live, tracked editable when static), hidden toggle (GM), current-turn highlight; event rows show
name + remaining lifespan. Controls: start/end, roll all/selected, advance, rewind (GM), end my
turn (owner), add combatant from selected tokens / add event (name, lifespan, message, hidden),
reorder (GM, touch drag with 44 px handles). Settings sub-panel: resource-registry editor and
world/scene override editor. All chrome i18n-keyed; reflows to phone width. UI gating is
advisory (`canEdit`); the server decides.

## 8. Effects lifecycle (D1, D16)

Expiry per §5; `combat:effect-expired` lets the system react (remove, notify). `effect-tick`
fires at each boundary an effect with a duration passes. An effect created while its host is a
combatant in an active combat gets `started` stamped by the server on Create; otherwise the
client stamps it when the host joins a combat. Nightfox migrates `active`/`transfer` from
`system.mechanics` to the engine band in its own repo after M14a lands (follow-up, logged).

## 9. Cost unification (D15)

One `step_cost(rule, parity, terrain_multiplier)` in the pathfinding module consumed by BOTH the
router and `execute_move`; `MoveOutcome.cost` and the router's preview cost are then the same
number for the same route. `los_smooth` recomputes each straightened chord's cost exactly
(per-span integration over the cells it crosses). Parity test: for each `DiagonalRule`, route a
path, execute it, assert equal cost; mutate one side and confirm the test fails.

## 10. Security & permissions

- All combat intents authz-checked server-side (GM / combatant owner / turn owner); UI advisory.
- Hidden entries: stripped at egress, `turn` projected, roll posts whispered, indistinguishable
  rejection — no observable difference between "hidden combatant exists" and "none". Hidden
  budgets never leave the GM tier (the whole-move-scalar rule).
- `Formula::Text` is opaque server-side and bounded client-side by the formula library's caps.
- Movement rejection carries no budget value to a non-owner (`NotYourTurn` only).
- The M13f schema registry continues to govern combatant/effect `system` bands.

## 11. Testing

- Server: transition matrix (advance, wrap, hidden auto-skip vs `GmOnly` hold, event fire/
  lifespan/removal, recovery per boundary numeric-only, expiry per `expires`×`unit`×`anchor`,
  loop guard); authz matrix per intent; movement gate (`Hard` truncation at exact affordable
  prefix, `Warn`/`None` decrement-only, GM exempt-from-truncation-not-decrement, `NotYourTurn`,
  `BudgetUnresolvable` on missing `perCell`); redaction (hidden absent from Welcome/broadcast/
  resync for players, present for GM; `turn` projected; indistinguishable error); one-active-per-
  scene; D15 parity per rule + navmesh.
- Client: hook emission from deltas (player and GM see identical visible events); `resolveResources`
  mirror/tracked semantics with a fixture system (movement + actions registry); Zod/ts-rs drift;
  tracker component tests; route-preview clamp/overage.
- E2E (two browsers): GM starts combat and rolls; player ends own turn; hidden NPC auto-resolves
  invisibly; move truncated under `Hard`; event posts to chat and expires; effect expires at the
  right boundary.

## 12. Checkpoints

| Checkpoint | Contents |
|---|---|
| **M14a** | Documents: `combat`, `combatant`, `resource-registry`, `effect` engine band, `CombatDefaults` on world/scene; ts-rs + Zod; registry seed helper; one-active-per-scene enforcement; hidden-entry egress redaction |
| **M14b** | Intents + transitions (§5), `chat::rolls` shared entry, movement gate, D15 cost unification, `TODO.md` closures |
| **M14c** | `AppContext.combat`, `CoreHooks` first entries + delta-derived emission, `resolveResources`, route-preview budget UX |
| **M14d** | `src/modules/combat` tracker + settings editors, e2e, docs-site module page, skill-update gate (new `shadowcat-codebase-combat` skill) |

Each checkpoint: plan → execute per project conventions; buddy-check pre-authorization
recommended for the M14b transition + gate code and the M14a redaction path.
