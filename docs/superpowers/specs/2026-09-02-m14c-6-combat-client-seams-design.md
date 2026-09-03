# M14c-6 · Combat Client Seams — Design

**Status:** Approved by fork-resolution (design agent, 2026-09-02; the user was unavailable for
a brainstorming dialogue, so every fork below is decided by "what is the best long-term shape
in keeping with our plans and goals?" and recorded with alternatives — §9). Sixth and last of the
six M14c sub-projects ([M14c-1 design](2026-08-30-m14c-1-server-formula-engine-design.md) §1).
Completes the [M14 design](2026-08-28-m14-combat-tracker-design.md) §6 ("Client seams") on the
server-evaluated model the M14c-1/-2 specs established — where §6 of that spec names
`resolveResources` or `remainingMovement`, this spec supersedes it (§9 S2, S9); the
[M14b design](2026-08-28-m14b-combat-clock-design.md) §1's "excludes (M14c/d)" list is delivered
here except for the tracker/editors, which are [M14d](2026-09-02-m14d-tracker-module-settings-editors-design.md).

## 0. What exists and what is missing

Built (M14a–M14c-5): the five combat doc types, `CombatDefaults` chain + `resolve_combat_rules`,
the eight `ClientMsg::Combat*` intents through `combat::handle_combat_intent`, the movement-budget
gate + `Hard` route-preview clamp (`PathResult.truncated`), server-side formula evaluation
(`combat::eval`), the `owner_or_gm` default on `/engine/resources`, and — client-side —
`WsClient.combat()`, the Zod mirrors of every combat frame/type, the document builders
(`buildCombatDoc`/`buildCombatantDoc`/`buildEffectDoc`/`buildCombatHistoryDoc`), and
`resolveSettingProvenance`.

Missing, and delivered here:

1. **`AppContext.combat`** — nothing on the client dispatches a combat intent or composes the
   combat documents into a usable model; every consumer would re-derive order/turn/authorization
   affordances from raw documents.
2. **The first `CoreHooks` entries** — `CoreHooks` is still the empty declaration-merge seam from
   the module system; no first-party hook exists, and combat events (M14 D5: derived from applied
   command deltas, identical on every client) have no emitter.
3. **Resolved resource numbers on the client** — M14c-2 C2 made `max` (Tracked) and `current`/`max`
   (Mirror) evaluate-on-use with no stored home, and its §5 explicitly parked "where a number is
   needed" on this sub-project. Today the only way a client could show a Mirror value or a Tracked
   ceiling is to evaluate the formula itself over its redacted store — the shape the campaign is
   removing.
4. **The `Warn` overage label** — `Enforcement::Warn` has no observable effect anywhere: the server
   decrements after the move and neither clamps nor refuses, and the route preview shows only
   `cost`/`arrested`. M14 D6: "`Warn` (route preview shows the overage during drag)".
5. **A defect in the existing seam** — `WsClient.combat()` confirms success by resolving the
   OLDEST pending combat entry on ANY self-authored broadcast `event`. A GM dragging a token
   (ordinary intents) while a `combat_advance` is in flight resolves that promise on the token
   write's echo; a `combat_error` arriving afterwards finds no entry and the rejection is lost.
   The realtime-sync skill records this as a "known failure mode"; under the campaign's iron rule
   it is fixed here, in range.
6. **`docs/site/protocol.md`** lists no combat frame at all (a pre-existing doc gap from M14b);
   fixed here alongside the frames this spec adds.

## 1. Scope

In: everything in §0. Server: one new reply frame, one new `PathResult` field, one new derived
channel. Client core: `CombatController`/`CombatApi`, `CoreHooks` entries + delta derivation +
emission, `WsClient.combat()` correlation rewrite, Zod mirrors. Shell: wiring into `WorldSession`/
`Table`/`AppContext`, host-provided service. scene-tools: the overage label. Docs + skills.

Out: the tracker panel, the settings/registry editors, e2e through the browser UI (M14d).
No change to any combat transition, gate, or document shape.

## 2. Decisions

| # | Decision |
|---|---|
| S1 | **`CombatController` is a framework-neutral class in `@shadowcat/core`** (`combat.ts`), exposed to Svelte modules as `AppContext.combat: CombatApi` and to every module as the host-provided service `COMBAT_SERVICE = "shadowcat.service:combat"` (M14 D14/D17). It imports no Svelte and holds no rune; reactivity reaches Svelte through the same `createSubscriber` bridge `ctx.documents` uses. |
| S2 | **Resolved resource numbers are server-derived, per recipient, over a new `"combat"` scene-derived channel** (`compute_derived` arm beside `"footprints"`), computed through the SAME `combat::eval::resolved_resource` the transitions and the movement gate use. The client evaluates nothing and stores nothing. |
| S3 | **A combat intent gets a correlated success reply: `ServerMsg::CombatResult { request_id, seq }`**, addressed to the originator only, sent after `Room::commit_combat` returns. `WsClient.combat()` resolves by `request_id` once the `event` carrying `seq` has been applied (`nextExpected > seq`); the author-FIFO path and `WsClientOptions.selfUserId` are deleted. |
| S4 | **Nine first-party hooks, all `kind: "info"`, version `1.0.0`**, declared by the host at session construction (`defineCombatHooks(hooks)`): `combat:start`, `combat:end`, `combat:round-start`, `combat:round-end`, `combat:turn-start`, `combat:turn-end`, `combat:rewind`, `combat:effect-tick`, `combat:effect-expired`. Payloads carry ids and scalars only (§5.1). |
| S5 | **Emission derives from the authoritative `DocumentStore`, never the optimistic view**, from pre-images captured in `WorldSession`'s `onCommand` before `store.applyCommand`. Exactly once per applied command; nothing from `seedDocuments`, from an optimistic prediction, from a `reject`, or from a resync-suppressed duplicate. Emission is sequential (a promise queue), so listeners observe commands in seq order. |
| S6 | **Turn boundaries derive from the server-recorded `combat-history` records the command appended, never from a client-side re-derivation of the server's walk.** `transition::enter_turn` already records every boundary `settle_turn` crosses (an `Event` of any lifespan, a hidden combatant's auto-resolve under `OwnerMayEnd`), folded into the one history write the same command carries; `deriveCombatHookEvents` walks the records placed after the pre-command current record (`crossedRecords`) and emits `turn-end`/round pairs/`turn-start` per boundary. The history document stays GM-only egress (M14b B8), so a player's derivation holds no records and falls back to the combat document's own endpoints (`turnWalk`): a moved `turn`, or the same `turn` in a later `round` (a lap), each a boundary by the clock's own definition. A GM therefore observes every crossed turn — a `lifespan: null` event's, a hidden combatant's (with `kind` resolved from the GM's own store) — and a player the endpoints only; no recipient infers a turn from a `lifespan` decrement or a combatant delete. |
| S7 | **Effect hooks require a combat-document op in the same command** (`Update` or `Delete` of a `combat`): that is the evidence the effect change is a clock transition. A GM's manual `active: false` outside a transition is an edit, not an expiry. |
| S8 | **An unresolvable `turn` (a hidden combatant holding the clock under `GmOnly`) still emits `turn-start`/`turn-end` with `combatantId` set and `kind: null`** — the id is already on the wire in `combat.turn`; a null `kind` is exactly "you cannot resolve this row". |
| S9 | **`Warn` is a client rendering over a server number: `PathResult.budget_cells: Option<f64>`**, the requester's remaining movement budget in cells for the named token, derived from `resolve_budget`'s own `decrement` half through a shared `budget_cells(current, cost_to_resource)` helper the `Hard` clamp ALSO uses; present iff the caller can READ the combatant (`BudgetGate::enforced`), regardless of enforcement mode. The client renders an overage label under `warn`, a truncation marker under `hard`, nothing under `none`. `AppContext.combat.remainingMovement(tokenId)` (M14 §6) is dropped — the preview reads `budget_cells`, the tracker reads the channel's `movement_cells`. |
| S10 | **Combat and combatant documents are client-created through ordinary intents**; the clock fields (`active`, `round`, `turn`) and the snapshot fields are never written by the client. A new combat is built by `newCombatEngine(sceneId)` at the engine defaults `transition::start` overwrites on the first start; the defaults are pinned cross-language by a JSON fixture both suites read (§4.4). |
| S11 | **Removing the current-turn combatant is refused client-side** (`CombatClientError`); the GM advances first. Writing `turn` from the client would make the clock two-owned. |
| S12 | **`addCombatants` is ONE intent**: the `Create`s plus one `/engine/order` append with the real OCC pre-image; `owner` is stamped from `effectiveOwner` (token owner, else the linked actor's). Actor-only combatants (no token) are allowed. |
| S13 | **`CombatApi.canAct` is advisory**, a mirror of `combat::authorize` for hiding controls; the server decides. |
| S14 | **`docs/site/protocol.md` gains the combat frames** (the eight intents, `combat_error`, `combat_result`, the `combat` channel, `path_result.budget_cells`) in range. |

## 3. Server

### 3.1 `ServerMsg::CombatResult` (S3)

```rust
/// A combat intent was accepted and committed as the sequenced `Event` at `seq`.
/// Addressed to the originating connection only; never broadcast. The broadcast
/// `Event` remains the state notification — this frame only correlates it.
CombatResult { request_id: Uuid, seq: i64 }
```

`combat::run_intent` already receives the committed `Command` from `Room::commit_combat`;
`handle_combat_intent` returns `Some(ServerMsg::CombatResult { request_id, seq: cmd.seq })` on
success (today `None`). The `ws::conn` dispatch arm is unchanged (it already forwards a `Some`
to the originator via `etx`). Egress ordering: `egress_loop` is `biased;` on `erx`, so the
`CombatResult` may reach the client BEFORE the `Event` at `seq` — the client handles both orders
(§4.1). Rate limiting, authorization and error handling are untouched.

### 3.2 `PathResult.budget_cells` (S9)

- `ws::room::resource_cells(current: f64, cost_to_resource: f64) -> f64` — the one division
  `resolve_budget`'s `Hard` ceiling already performs (`n.current / ctr`), extracted so the clamp
  and the preview number are the same arithmetic. `resolve_budget` calls it for `budget_cells`;
  `ResolvedBudget` gains nothing (it already carries `current` and `cost_to_resource`).
- `handle_pathfind`: when the gate resolves (`BudgetResolution::Resolved { decrement: Some(d), .. }`)
  and `bg.enforced` (the caller can READ the combatant — the same predicate that gates refusal and
  truncation), the reply carries `budget_cells: Some(resource_cells(d.current, d.cost_to_resource))` (a
  NEW `handle_pathfind` local, `reply_budget_cells`, distinct from the existing `Hard`-only
  truncation-ceiling local `budget_cells` that feeds `SceneEcs::pathfind` — that local and its
  `Hard`-only population are unchanged);
  otherwise `None`. A GM is `enforced` (a GM reads every document), so a GM preview shows the
  number too — their move still decrements. `NotYourTurn`/`Unresolvable` refusals are unchanged
  (generic `PathError`). Under `Hard` the route is truncated AND `budget_cells` is present.
- Disclosure: identical to the clamp's — only a caller who already receives the combatant
  document (and, being enforced, its `/engine/resources` band as owner or GM) receives the
  number. A non-reader gets `None`, indistinguishable from "no combat".
- ts-rs regeneration; the client `PathResult` type + Zod schema gain `budget_cells: number | null`.

### 3.3 The `"combat"` derived channel (S2)

`combat::channel` (new unit, ts-rs exported types; the payload builder lives on `SceneEcs`
beside `resolved_footprints`, since it reads the ECS's `combats` side table, the combatant
scene entities, `actors`, and the cached `resource-registry`):

```rust
CombatsPayload  { combats: Vec<CombatView> }                 // sorted by combat id (stable fingerprint)
CombatView      { id: Uuid, scene_id: Uuid, combatants: Vec<CombatantView> }  // sorted by combatant id
CombatantView   { id: Uuid,
                  resources: Option<BTreeMap<String, ResolvedResourceView>>,   // None = band not visible to ctx
                  movement_cells: Option<f64> }              // None = no movement resource / unresolvable / band hidden
ResolvedResourceView { binding: ResourceBindingKind /* "mirror" | "tracked" */,
                       current: Option<f64>, max: Option<f64>,
                       error: Option<String> }               // FormulaError detail when evaluation failed
```

`SceneEcs::resolved_combats(ctx, world_defaults) -> CombatsPayload`:

- A combat is included iff `ctx` may READ its document (`ctx_access(...).has(cap::READ)`); a
  combatant iff `ctx` may READ it — a hidden combatant is ABSENT, never a placeholder (the D9
  rule the document stream already applies).
- `resources` is `Some` iff `ctx` may see the `/engine/resources` pointer on that combatant —
  resolved exactly as document egress resolves it (the document's `property_overrides` entry for
  that pointer, else `Visibility::All`, tested through `Access::can_see`). Otherwise `None` and
  `movement_cells: None`.
- Every registry key is resolved for every included combatant through
  `combat::eval::resolved_resource(binding, stored, host)` with `host =
  SceneEcs::combatant_formula_host(&kind)` — the same call `resolve_budget` makes. Mirror ⇒
  `current = max = eval(value)`; Tracked ⇒ `max = eval(max)`, `current = stored ?? max`
  (lazy-full). An `Err` becomes `{ current: None, max: None, error: Some(detail) }`.
- `movement_cells`: when the combat's `movement.resource` names a Tracked key that resolved,
  `resource_cells(current, cost_to_resource)` with `cost_to_resource` = `scene_per_cell(scene)` under
  `PerCell` (`None` ⇒ `None`) or `1.0` under `Spaces` — the same conversion `resolve_budget` uses,
  through the same helper. Computed for every combat, active or not (a paused combat's rows still
  show budgets).
- `compute_derived`: `"combat" => serde_json::to_value(ecs.resolved_combats(ctx, world_defaults)).ok()`.
  The existing subscription machinery (150 ms leading-edge debounce after events, whole-payload
  fingerprint suppression, GM see-as via `as_user`) applies unchanged.
- `SceneEcs` already caches `combats`, combatants (scene entities), `actors`, the registry and
  the scene `grid.distance` — no new hydration.

Anti-drift test: for a fixture combat, the channel's `movement_cells` for the turn owner equals
the `budget_cells` a `Hard` `resolve_budget` computes for the same token — both through the one
helper; sabotage either side and the test fails.

## 4. Client core (`@shadowcat/core`)

### 4.1 `WsClient.combat()` correlation (S3)

- `combatPending` stays keyed by `request_id`; each entry gains `seq: number | null`.
- `case "combat_result"`: look up the entry; if `msg.seq < this.nextExpected` (the event is
  already applied) resolve now; else store `seq` on the entry and leave it.
- `applyEvent`: after dispatching an in-order command and advancing `nextExpected`, resolve every
  entry whose stored `seq` is now `< nextExpected` (at most one in practice; a loop keeps it
  correct).
- `case "combat_error"`: unchanged (reject by `request_id`).
- The `case "event"` author-FIFO block and `WsClientOptions.selfUserId` are deleted;
  `WorldSession` stops passing `selfUserId`. Timeout and `failPending` behaviour unchanged.
- The resolved value is `void` — the store already reflects the event by the time the promise
  settles, which is the property a tracker relies on ("after `await advance()`, `turnOf()` is the
  new turn").
- Zod: `ServerMsgSchema` gains the `combat_result` variant (`request_id: string`, `seq: int`).

### 4.2 `CombatController` / `CombatApi` (S1, S10–S13)

`src/client/core/src/combat.ts`. Exports: `COMBAT_SERVICE`, `CombatApi`, `CombatController`,
`CombatControllerDeps`, `CombatAffordances`, `NewCombatant`, `NewEvent`, `CombatClientError`,
`CombatsView`/`CombatView`/`CombatantView`/`ResolvedResourceView` (the parsed channel types, Zod
`CombatsPayloadSchema` + `parseCombats` in `wire.ts` beside `parseFootprints`'s home),
`EMPTY_COMBATS`, `newCombatEngine`, `ENGINE_COMBAT_DEFAULTS` (moved out of `scene-docs.ts`'s
private scope and exported — §4.4 pins it).

```ts
export interface CombatControllerDeps {
  documents: ReadableDocuments;                    // the optimistic view — reads
  dispatchIntent: (ops: WireOperation[]) => void;  // document helpers
  sendCombat: (msg: Extract<ClientMsg, { type: `combat_${string}` }>) => Promise<void>;
  selfId: string;
  role: () => WorldRole | null;                    // live — Welcome sets it after construction
  canEdit: (doc: WireDocument, path: string) => boolean;
  logger: Logger;
}

export interface CombatApi {
  // ---- reads (over `documents`; a hidden combatant is simply absent) ----
  combatsFor(sceneId: string): WireDocument[];   // active first, then by id
  activeFor(sceneId: string): WireDocument | null;
  combatants(combatId: string): WireDocument[];  // in `engine.order`; ids the store cannot resolve are skipped;
                                                 // parented combatants absent from `order` are appended (id order)
  turnOf(combatId: string): WireDocument | null; // null when no turn or unresolvable (§2 S8)
  readonly resolved: CombatsView;                // latest `"combat"` frame; EMPTY_COMBATS before the first
  resolvedFor(combatantId: string): CombatantView | null;
  subscribe(listener: () => void): () => void;   // resolved-frame changes only (document changes flow through `documents.subscribe`)
  canAct(combatId: string): CombatAffordances;   // advisory (S13)
  // ---- intents: the server-owned clock; reject with the server's player-presentable message ----
  start(combatId: string): Promise<void>;
  pause(combatId: string): Promise<void>;
  end(combatId: string): Promise<void>;
  advance(combatId: string): Promise<void>;
  rewind(combatId: string): Promise<void>;
  sort(combatId: string): Promise<void>;
  roll(combatId: string, channel: string, rolls: WireCombatRollEntry[]): Promise<void>;
  modifyResource(combatId: string, combatantId: string, resource: string, op: WireResourceOp): Promise<void>;
  // ---- document helpers: optimistic intents through `dispatchIntent` ----
  createCombat(sceneId: string, opts?: { name?: string | null; id?: string }): string;
  deleteCombat(combatId: string): void;          // an inactive combat; an active one goes through `end()`
  addCombatants(combatId: string, entries: NewCombatant[]): string[];   // one intent (S12)
  addEvent(combatId: string, ev: NewEvent): string;
  removeCombatant(combatId: string, combatantId: string): void;         // throws CombatClientError on the current turn (S11)
  setHidden(combatantId: string, hidden: boolean): void;
  reorder(combatId: string, order: string[]): void;                     // same id set, new sequence; else CombatClientError
  setInitiative(combatantId: string, initiative: number | null, tiebreak?: number): void;
}

export interface NewCombatant { tokenId?: string; actorId?: string; hidden?: boolean; name?: string | null; system?: unknown; }
export interface NewEvent     { name: string; lifespan: number | null; message: string | null; hidden?: boolean; system?: unknown; }
export interface CombatAffordances {
  start: boolean; pause: boolean; end: boolean; advance: boolean; rewind: boolean; sort: boolean;
  edit: boolean;                               // add/remove/reorder/hide/delete — GM
  roll(combatantId: string): boolean;          // GM, or owner + canEdit(doc, "/engine")
  resource(combatantId: string): boolean;      // same rule as roll
}
```

Semantics:

- **Intents** build the frame with a fresh `crypto.randomUUID()` `request_id` and call
  `sendCombat`. Rejections propagate (the tracker surfaces them through `ctx.notify`).
- **`createCombat`**: `buildCombatDoc(world, newCombatEngine(sceneId), id)` with `name`;
  `newCombatEngine` = `{ scene_id, active: false, round: 0, turn: null, order: [],
  turn_control, movement: { resource, interpretation, enforcement }, effect_cleanup,
  rewind_restore, forward_restore, effect_lifecycle }` at `ENGINE_COMBAT_DEFAULTS`. The world id
  comes from the first document's scope — the controller takes `world` in deps (add
  `world: string` to `CombatControllerDeps`).
- **`addCombatants`**: per entry resolves the token (`documents.get(tokenId)`) and/or actor;
  `kind: { type: "actor", token_id, actor_id }` where `actor_id` is the explicit `actorId`, else
  the token's `engine.actor_id`, else `null`; at least one of the two non-null or
  `CombatClientError`. `name` defaults to the token's `name`, else the actor's. `owner =
  effectiveOwner(token ?? actor, documents)`. Engine `{ kind, initiative: null, tiebreak: 0,
  resources: {} }`. Builds through `buildCombatantDoc` (which stamps the `owner_or_gm`
  resources override and the visible-owner `users` grant). Ops: the `create`s, then ONE `update`
  on the combat's `/engine/order` with `old = current order`, `new = [...current, ...newIds]`.
- **`addEvent`**: `kind: { type: "event", lifespan, message }`, `name`, `hidden`, no owner; same
  order append.
- **`removeCombatant`**: refuses when `combat.engine.turn === combatantId` (S11); else ONE
  intent: `update` `/engine/order` (filtered) + `delete` of the combatant (the pre-image doc from
  the store).
- **`setHidden`**: `/permissions/default` `"observer"`↔`"none"` and the owner's
  `/permissions/users/<owner>` entry (`"owner"` when visible; `remove: true` when hidden), each
  with its real pre-image — the D9 shape `buildCombatantDoc` produces at creation.
- **`reorder`**: validates the multiset equality against the current `order`, writes
  `/engine/order`.
- **`setInitiative`**: `/engine/initiative` (+ `/engine/tiebreak` when given), pre-images from
  the doc.
- **`canAct`** mirrors `combat::authorize`: GM ⇒ everything; a non-GM: `advance` iff the
  combat's `turn_control === "owner_may_end"` and `turnOf(combat)?.owner === selfId`;
  `roll(c)`/`resource(c)` iff the combatant's `owner === selfId` and `canEdit(doc, "/engine")`;
  all else `false`. `start` additionally requires `order.length > 0`; `rewind` requires
  `round > 0`.
- **`resolved`/`subscribe`**: `WorldSession` feeds parsed `"combat"` frames via
  `controller.setResolved(view)` (a method on the class, not on `CombatApi`); listeners fire on
  each replacement.

### 4.3 Hooks (S4–S8)

`src/client/core/src/combat-hooks.ts`:

```ts
declare module "./hooks" {
  interface CoreHooks {
    "combat:start":          { combatId: string; sceneId: string; round: number; resumed: boolean };
    "combat:end":            { combatId: string; sceneId: string; round: number; reason: "paused" | "ended" };
    "combat:round-start":    { combatId: string; round: number };
    "combat:round-end":      { combatId: string; round: number };
    "combat:turn-start":     { combatId: string; round: number; combatantId: string; kind: "actor" | "event" | null };
    "combat:turn-end":       { combatId: string; round: number; combatantId: string; kind: "actor" | "event" | null };
    "combat:rewind":         { combatId: string; round: number; turn: string | null };
    "combat:effect-tick":    { combatId: string; round: number; hostId: string; path: string; effectId: string | null; remaining: number };
    "combat:effect-expired": { combatId: string; round: number; hostId: string; path: string; effectId: string | null };
  }
}
export const COMBAT_HOOK_VERSION = "1.0.0";
export function defineCombatHooks(hooks: HookBus): void;   // defineHook for all nine, kind "info"
export type CombatHookEvent = { [K in keyof CoreHooks]: { name: K; payload: CoreHooks[K] } }[keyof CoreHooks];
export function deriveCombatHookEvents(before: (id: string) => WireDocument | undefined, cmd: WireCommand, after: ReadableDocuments): CombatHookEvent[];
export class CombatHookEmitter { constructor(hooks: HookBus, logger: Logger); emit(events: CombatHookEvent[]): void; }
```

**Derivation** (`deriveCombatHookEvents`, pure, table-driven):

1. Collect the combat docs the command touches: `update` ops whose `before(doc_id)` is a
   `combat`, `create` ops with `doc.doc_type === "combat"`, `delete` ops with
   `doc.doc_type === "combat"`. For each, `b = before(...)?.engine`, `a = after.get(id)?.engine`
   (a deleted combat has `a = undefined`). No combat op ⇒ no events at all (including effect
   events — S7).
2. Per combat, in this order:
   - **Rewind classification**: `a` present and `b` present and (`a.round < b.round` or
     (`a.round === b.round` and both `b.turn`, `a.turn` index in `a.order` and
     `idx(a.turn) < idx(b.turn)`)) ⇒ emit `combat:rewind { round: a.round, turn: a.turn }` and
     STOP for this combat (no round/turn/effect events).
   - **Start**: (`b` absent or `!b.active`) and `a?.active` ⇒ `combat:start { resumed: b?.turn != null }`
     with `sceneId = a.scene_id`, `round = a.round`.
   - **The turn walk** (S6): the ordered boundaries the command crossed, `turnWalk`. When the
     command carries a visible `combat-history` op for this combat (`collectHistoryTouch`: a
     `create` parented to it, or an `update` whose pre-image is its history), the walk is the
     records between the pre-command current record and the new cursor (`crossedRecords`: the
     pre-command current record is located by `(round, turn)` identity scanning down from its
     old index, since eviction only shifts it left and no record the same command appends can
     share its identity; a fast-forward, which moves `cursor` over existing records, crosses
     exactly those; an unlocatable record falls back to the endpoints). Otherwise — every
     player, and any GM command that moved the clock without a history write — the walk is the
     one boundary the combat document evidences: `a.turn` non-null and (`b` absent, or
     `a.turn !== b.turn`, or `a.round !== b.round`).
   - **Per boundary `{round, turn}`**, with `prev` starting at `{b.round, b.turn}` (or `{0,
     null}`): `combat:turn-end` for `prev.turn` when non-null (`round = prev.round`); then for
     `r` from `prev.round` to `round - 1`, `combat:round-end { round: r }` when `r > 0` and
     `combat:round-start { round: r + 1 }`; then `combat:turn-start` for `turn` (`round`);
     `prev` becomes the boundary. `kind` for either event resolves from `after.get(turn)` else
     `before(turn)` (an exhausted `Event` deleted in the same command), else `null` (a hidden
     combatant the recipient's store never held).
   - **Trailing**: when `a?.turn` is null and `prev.turn` is not (a deleted combat, or a cleared
     `turn`), `combat:turn-end` for `prev.turn`; then, when `a` is present, the round pairs from
     `prev.round` to `a.round`.
   - **End**: (`b?.active` and `a` present and `!a.active`) ⇒ `combat:end { reason: "paused" }`;
     `b?.active` and `a` absent (deleted) ⇒ `combat:end { reason: "ended" }`. `sceneId`/`round`
     from `b`.
3. **Effect events** (S7), after the per-combat events, attributed to the ONE combat the command
   touched (when several — a `CombatStart` also pausing another — the one whose `a?.active` is
   true, else the first): every `update` op on any doc with a change whose path matches
   `^((?:/embedded/[^/]+/\d+)+)/engine/(duration/remaining|active)$`:
   - `…/duration/remaining` with `new` a number and (`old == null` or `new < old`) ⇒
     `combat:effect-tick { hostId, path: <embedded prefix>, effectId, remaining: new }`;
   - `…/engine/active` with `old === true` and `new === false` ⇒ `combat:effect-expired`.
   `effectId` = the embedded child's `id` at the prefix in `after.get(hostId)` (else `before`),
   `null` when the host is not resolvable. `round` = the attributed combat's `a?.round ?? b.round`.
4. Ordering guarantees (documented on the function): per command — rewind XOR (start → per
   crossed boundary [turn-end of the turn being left → round-end/round-start pairs → turn-start]
   → trailing turn-end/rounds → end) → effect events; across commands — seq order (the emitter's
   queue).

**Per-recipient safety**: a non-GM's store never holds a hidden combatant or the history doc,
so their payloads never name one except through `combat.turn` (S8) — the same disclosure the
document stream already makes. Every client, GM or not, derives from ITS OWN filtered stream, so
two GMs see identical events and a player sees the subset its documents admit (M14 D5): the
history records reach the GM alone, so a GM's list carries every crossed boundary and a
player's the endpoints — a subsequence, never a different clock.

**Emission** (`CombatHookEmitter.emit`): chains `hooks.emitInfo(name, payload)` calls onto an
internal promise queue (`this.#tail = this.#tail.then(...)`), awaiting each before the next, so
a listener that awaits never sees a later command's event before an earlier one's; a thrown
listener is already isolated by `HookBus`. The queue is fire-and-forget from `onCommand`.

### 4.4 Engine-defaults fixture (S10)

`src/client/core/src/__fixtures__/engine-combat-defaults.json` — the JSON of
`ResolvedCombatRules` at `resolve_combat_rules(None, None, None)` in the client's camelCase
`CombatDefaults` spelling. A Rust test (`data::engine::combat::tests`) serializes the resolved
rules into that spelling and asserts byte-equality with the fixture; a Vitest test asserts
`ENGINE_COMBAT_DEFAULTS` deep-equals it. Either side drifting fails a test (the M14c-1 corpus
pattern).

## 5. Shell (`@shadowcat/shell`) and ui-kit

### 5.1 `WorldSession`

- Constructor: keeps a reference to the `HookBus` and `ServiceRegistry` it builds for the
  `ModuleRegistry`; calls `defineCombatHooks(hooks)`; constructs `#combat = new CombatController({...})`
  with `sendCombat: (m) => this.#ws ? this.#ws.combat(m) : Promise.reject(new Error("not connected"))`
  and `role: () => this.role`; provides it: `services.provide(COMBAT_SERVICE, this.#combat,
  { version: COMBAT_HOOK_VERSION })` (host-provided, no `module`). Exposes `get combat():
  CombatApi`.
- `onCommand`: captures pre-images for the ids the command touches (`update` → `store.get(doc_id)`;
  `delete` → the op's own `doc`; `create` → none) into a `Map`, applies to both mirrors as
  today, then `this.#combatEmitter.emit(deriveCombatHookEvents((id) => before.get(id), cmd,
  this.store))`. The map is built only when the command touches a `combat`/`combatant` doc or
  an embedded-effect path — a cheap pre-scan (`commandTouchesCombat(cmd, store)` exported from
  `combat-hooks.ts`), so ordinary token drags pay nothing.
- `enter()`: beside the footprints subscription, `#combatSub = this.subscribeScene("combat",
  (f) => this.#combat.setResolved(parseCombats(f.payload)))`; `leave()` drops it and resets to
  `EMPTY_COMBATS`.
- `WsClient` construction: `selfUserId` removed.

### 5.2 `AppContext`

`combat: CombatApi` (doc: "The combat seam: reads over the per-recipient optimistic view,
server-resolved resource numbers, the server-owned clock's intents, and the document helpers;
see `CombatApi`. Advisory affordances only — the server authorizes every intent."). `Table`
wires `combat: session.combat`. `setAppContextForTest` defaults `combat` to a
`CombatController` over the fixture's `documents` with a rejecting `sendCombat` — so existing
component tests keep compiling and a test can drive the real controller.

### 5.3 scene-tools (S9)

`requestRoute`'s success branch: `const enforcement = ctx.combat.activeFor(scene.id)?.engine.movement.enforcement`;
with `result.budget_cells != null`:

- `"warn"` and `result.cost > result.budget_cells`: label
  `${budgetLabel} · ${t("tools.overBudget", { over: formatCellDistance(result.cost - result.budget_cells, scene) })}`
  ("35 ft · 10 ft over budget") and the route polyline stroke switches to `ROUTE_WARN_COLOR`.
- `"hard"` and `result.truncated`: label `${budgetLabel} · ${t("tools.budgetStop")}` ("stops at
  budget"); polyline unchanged (the server already cut it).
- `"none"` or no active combat: unchanged.
- The `arrested` marker composes with either (`⚠` stays a suffix).

New i18n keys `tools.overBudget`, `tools.budgetStop` in `en.ts`. The move itself is unchanged
(the server decrements; under `warn` an over-budget move executes in full).

## 6. Wire summary

| Change | Shape | Direction |
|---|---|---|
| `combat_result` | `{ request_id, seq }` | server → originator |
| `path_result.budget_cells` | `number \| null` | server → requester |
| `scene_derived` channel `"combat"` | `CombatsPayload` | server → subscriber (per recipient) |
| `WsClientOptions.selfUserId` | deleted | client-internal |

No `ClientMsg` change. ts-rs regeneration + committed bindings; Zod mirrors + the drift guard.

## 7. Testing

**Server** (`ws/conn/tests/combat_intents.rs`, `ws/room/tests/movement_budget.rs`, new
`combat/channel/tests.rs`, `scene/tests/combat_index.rs`):
- `CombatResult { seq }` reaches the originator and equals the broadcast `Event`'s seq; a
  second connection never receives it; a rejection still yields only `CombatError`.
- `budget_cells`: owner under `Warn` gets the number and no truncation; owner under `Hard` gets
  number + `truncated`; GM gets the number; a non-reader (hidden combatant) gets `None`; a
  non-combatant token gets `None`; `Spaces` vs `PerCell` conversion; the shared-helper parity
  test (§3.3) with a recorded sabotage.
- Channel: GM sees every combat/combatant with numbers; the owner sees own numbers and
  `resources: null` on another player's combatant; a hidden combatant is absent for the player,
  present for the GM; Mirror/Tracked/lazy-full/eval-error shapes; `movement_cells` under both
  interpretations and `None` without `grid.distance`; a paused combat is still reported;
  payload stable across two computations (fingerprint).
- Doc gates (`-D missing-docs`), clippy, fmt.

**Client core** (`ws-client.test.ts`, `combat.test.ts`, `combat-hooks.test.ts`, `wire.test.ts`):
- `combat()` resolves only after the event at `seq` is applied, in both frame orders; a foreign
  self-authored event no longer resolves anything; `combat_error` rejects; timeout; `failPending`.
- Controller: every read over a fixture store (hidden absent, order/unresolvable turn, active
  first); every document helper's ops (exact paths, pre-images, single intent); `removeCombatant`
  on the turn throws; `reorder` set check; `canAct` matrix (GM / owner under `owner_may_end` /
  owner under `gm_only` / non-owner / observer-tier owner without write cap); intents build the
  frame and propagate rejection.
- Hooks: a table of (before, command, after) → expected event list covering start (initial +
  resume), advance within a round, wrap (round-end/round-start), event intermediate turn +
  lifespan removal, hidden-turn `kind: null`, pause, end-by-delete, rewind (both branches),
  effect tick (materializing `null → n` and `n → n-1`) and expiry, effect edit WITHOUT a combat
  op (nothing), ordering within a command, and a `CombatStart` swapping two combats. The
  emitter's queue preserves order across two synchronous `emit` calls. `deriveCombatHookEvents`
  ignores `create`-only commands with no combat.
- Fixture parity (§4.4).

**Shell** (`worldSession.test.ts`, `Table.test.ts`): pre-image capture + emission on a real
`DocumentStore`; no emission on `seedDocuments`, on an optimistic `applyIntent`, or on a
`reject`; the `"combat"` subscription re-establishes on Welcome and clears on `leave()`;
`COMBAT_SERVICE` resolvable via `ModuleContext.services.get`; `AppContext.combat` wired.

**scene-tools** (`measure-tool.test.ts`): the three label shapes and the color switch under a
fake `ctx.combat`.

**Core e2e** (`src/client/core/src/e2e/combat-seams.e2e.test.ts`, real `test_server`): GM creates
a combat + two combatants (one owned by the player) through raw intents, subscribes both clients
to `"combat"`, starts and advances; asserts (a) `CombatResult` correlation on the GM client,
(b) both clients derive identical `combat:*` event lists for the visible transition, (c) the
player's channel frame carries own numbers and `null` for the GM's combatant, (d) a hidden
combatant is absent from the player's frames and hook payloads, (e) the player's `pathfind`
reply carries `budget_cells` under `warn` for their own token.

## 8. Docs & skills

- `docs/site/protocol.md`: the eight `combat_*` intents, `combat_error`, `combat_result`, the
  `combat` channel under "Scene channels", `path_result`'s `budget_cells` (S14).
- Skills (plugin checkout, reviewed skill-update gate): `shadowcat-codebase-combat` (the client
  seams: `CombatController`, the hooks + derivation rules, the channel; the "nothing dispatches
  a combat intent from the UI" purpose line inverts), `shadowcat-codebase-client-shell`
  (`AppContext.combat`, the service, the `"combat"` session subscription beside footprints),
  `shadowcat-codebase-realtime-sync` (the third correlated family now resolves by
  `combat_result`; the `selfUserId` failure mode paragraph is deleted),
  `shadowcat-codebase-scene-rendering` (`PathResult.budget_cells`, the shared `resource_cells`
  helper).
- `docs/PLAN.md` M14c-6 DONE marker; `docs/HISTORY.md` delivery entry; POST_WORK_FINDINGS sweep.

## 9. Decision log

| Fork | Chosen | Alternatives and why not |
|---|---|---|
| Where the controller lives | `@shadowcat/core`, framework-neutral; AppContext + service both point at it (S1) | A ui-kit controller like `TemplatesController`: a Svelte-only seam contradicts invariant 7 and M14 D14's `shadowcat.service:combat`. |
| Source of resolved numbers | Server-derived `"combat"` channel (S2) | (a) Client evaluation over redacted docs — a second implementation of `resolved_resource` (never-fork), silently `unknown-ref` wherever a leaf is redacted, and the exact shape M14c-1 retired. (b) Server writes `max` back onto the combatant at each boundary — M14c-2 C2 forbids a second stored home, and it would chase every actor edit. |
| Success confirmation of a combat intent | `CombatResult { request_id, seq }` + resolve-after-apply (S3) | Keep the author-FIFO echo: mis-resolves on any self-authored event and can lose a rejection — a real defect in range, so the iron rule applies. Reusing `Event.intent_id` for combat commits: `OptimisticClient` treats an author-echo as a FIFO confirm of a PENDING intent, and a combat intent never enters `pending`, so a non-null `intent_id` would need a second correlation path inside the optimistic client — more machinery than a reply frame. |
| Hook set | The eight named in M14 §6 plus `combat:rewind` (S4) | Folding a rewind into `turn-start`/`round-start` would tell listeners the clock advanced when it moved backwards. |
| Hook payload shape | ids + scalars (`kind`, `round`, `resumed`, `reason`, `remaining`) | Whole documents in payloads would let a listener hold a stale copy and would tie the hook contract to the document shape. |
| Derivation source | Authoritative `DocumentStore` pre/post (S5) | The optimistic view emits predicted events that a rejection would have to un-emit — hooks are informational and cannot be retracted. |
| Intermediate turns (auto-resolved events of any lifespan, hidden auto-resolves, a same-actor lap) | Derived from the `combat-history` records the command appended; players fall back to the combat document's endpoints (S6) | (a) Inferring a turn from a `lifespan` decrement or a combatant delete: a second copy of `settle_turn`'s decision on the client (never-fork), blind to a `lifespan: null` event and to a lap the coalesced `/engine/turn` write folds to a same-value change. (b) A per-recipient-redacted history: the record captures every combatant's full bands, so a player-readable copy needs a new array-element redaction pass keyed on another document's permissions — new secrecy machinery M14b B8 chose GM-only egress to avoid. The player subset is a subsequence of the GM's list, not a different clock. |
| Effect events outside a transition | Not emitted (S7) | Treating every `active: false` as an expiry mislabels a GM's manual toggle. |
| Unresolvable `turn` | `kind: null` (S8) | Suppressing the turn hook hides that the clock moved, which `combat.turn` already discloses. |
| `Warn` number source | `PathResult.budget_cells` through the executor's own resolution (S9) | Client-side `current / perCell` from the channel: two arithmetic sites for one number (the clamp already has it) and a second interpretation switch. |
| `remainingMovement(tokenId)` | Dropped (S9) | It would re-implement `combatant_for_token`'s token→combatant join on the client (a third copy of that rule) for a number the preview and the channel already deliver. |
| `budget_cells` under `none` | Present (S9) | Omitting it under `none` adds a mode switch server-side for no disclosure gain — the number reaches only a reader of the combatant. |
| Combat creation | Client `Create` with engine-default placeholders + `start` snapshot (S10) | A `CombatCreate` intent: the server would only be relaying a document write a GM is already authorized to make; the placeholder fields are overwritten by the first `start` by design. |
| Removing the turn combatant | Client refusal (S11) | Writing `/engine/turn` from the client makes the clock two-owned; letting the server's `CombatEngine::validate` reject it turns a predictable UX rule into a generic "rejected". |
| `addCombatants` batching | One intent (S12) | Separate intents let a combatant exist outside `order` between them and double the OCC exposure on `/engine/order`. |

## 10. Open questions for the user

None. Every fork above resolved under "best long-term shape in keeping with our plans and
goals".
