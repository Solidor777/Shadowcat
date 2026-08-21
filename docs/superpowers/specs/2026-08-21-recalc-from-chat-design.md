# Recalc-From-Chat — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority —
see `docs/superpowers/specs/2026-08-13-debt-burndown-campaign-design.md` for the campaign's
"determine the best long-term shape, ask only when genuinely unanswerable" mandate).

**Spec for:** `docs/TODO.md` bucket-C sub-project 1, "Recalc-from-chat — persist `spec`/`raws` on
`RollEmbed` (persistence + secrecy fork)."

## 1. Scope

Today `Segment::RollEmbed { formula, outcome }` discards the parsed `RollSpec` and the natural-face
`RawRoll` the moment a roll is scored — `dice::recalc::recalculate` is a fully built, tested library
function with zero production callers. This sub-project (a) persists enough of a roll's intermediate
state that a later recalculation is possible and deterministic, and (b) wires a GM-triggered recalc
affordance from chat through to that library function, replacing a roll's outcome with an
auditable, visibly-marked correction rather than a silent rewrite.

Out of scope: a player-facing "redo my own roll" self-service affordance (see §3), and any change to
`dice::recalc` itself (already correct and tested).

## 2. Data model

### 2.1 `RollEmbed` gains two new optional fields, plus a stable identity and a recalc history

```rust
/// A dice roll embedded in a chat message.
pub struct RollEmbed {
    pub formula: String,
    pub outcome: RollOutcome,
    /// Stable identity for this roll, independent of its position in `content` — a recalc
    /// targets a roll by this id, never by array index, so it survives any future reordering
    /// (e.g. post-publish link-preview enrichment appending later segments).
    #[serde(default = "Uuid::new_v4")]
    pub roll_id: Uuid,
    /// The parsed formula this roll was scored from. `None` for any roll embedded before this
    /// field existed (an old stored document with no `spec` cannot be recalculated — its
    /// `recalculate` entry point refuses on a `None`, never guesses a spec back from `outcome`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<RollSpec>,
    /// The natural-face roll log this roll's `outcome` was evaluated from. Same
    /// `None`-for-pre-existing-rolls rule as `spec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawRoll>,
    /// Present iff this roll has been recalculated at least once. Carries every recalculation as
    /// an ordered, append-only log — the roll's original `outcome` is never silently discarded,
    /// so a viewer with recalc-visibility can always audit exactly what changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalc_history: Option<Vec<RecalcEntry>>,
}

/// One applied recalculation. `raw`/`outcome` are the PRE-recalc state this entry replaced —
/// the roll's live `raw`/`outcome` after the Nth entry is the Nth entry's *output*, which is
/// either the (N+1)th entry's `raw`/`outcome` or, for the last entry, the current `RollEmbed.raw`/
/// `RollEmbed.outcome`.
pub struct RecalcEntry {
    pub ops: Vec<RecalcOp>,
    pub previous_raw: RawRoll,
    pub previous_outcome: RollOutcome,
    pub recalculated_by: Uuid,
    pub recalculated_at: DateTime<Utc>,
}
```

`spec`/`raw`/`recalc_history` are `#[serde(default)]`-guarded exactly like `RollEmbed`'s sibling
optional fields elsewhere on `MessageEngine` — no SQL schema change, the whole thing rides
`documents.json` verbatim. Neither `RollSpec`/`RawRoll`/`RecalcOp` gains a `TS` derive (the dice
crate stays wire-free by design); the client mirrors only what it needs (see §4).

### 2.2 Producing `spec`/`raw` costs nothing new

`chat::rolls::execute_roll_with_seed` already constructs both values locally today and discards
them — it starts keeping them instead of dropping them, threading them into the `RollEmbed` it
returns. No pipeline change.

## 3. Authorization model — resolved design fork

**Recalc is GM-only, audience-independent, exactly mirroring `handle_edit_message`/
`handle_delete_message`'s existing moderation authority** — not a self-service affordance for the
roll's own author. This is the direct consequence of why rolls are edit-immutable in the first
place: immutability exists specifically to stop a player silently rewriting an inconvenient result.
A "recalc your own roll" self-service path reopens exactly that hole (a player rerolls until the
number they want appears) unless gated behind GM approval anyway — so there is no independent
self-service tier to design; GM-only is the correct and sufficient permission model, not a
simplification of a richer one.

This reuses the existing `WriteOrigin::ServerMessageRevision` chokepoint verbatim: a new
`handle_recalc_roll` handler runs the same "is this sender a GM in this room's world" check
`handle_edit_message` already runs, then publishes an `Operation::Update` under
`WriteOrigin::ServerMessageRevision` targeting the message document — a third caller of an
already-proven-sound chokepoint, not a new mechanism.

## 4. Wire exposure — resolved design fork

`spec`/`raw` are **GM-visible only**, reusing the existing per-field `property_overrides`
redaction machinery rather than inventing a bespoke recalc-specific filter (never fork a decision
across two paths that must agree on visibility). `data::validation::redaction_target` gains a new
recognized shape: a `RollEmbed` pointer's `spec` or `raw` (or `raws`, matching the recalc-op
addressing shape — see §5) leaf classifies as `gm_only`, the same band `filter_properties`/
`collect_hidden` already enforce for every other GM-only field, at any embedded depth. A message's
`content` array is embedded data (not a first-class document reference), so this is the same
"classify a pointer that lands inside untyped/embedded JSON" case the classifier already handles
for every other embedded band — no new classifier code shape, one new recognized field-name pair.
`recalc_history` and `roll_id` are **not** GM-only: `recalc_history` is what makes a recalculated
roll visibly auditable to every recipient who could already see the message (see §5), and `roll_id`
is a stable identifier with no secrecy value.

Non-GM recipients continue to see exactly what they see today: `formula`, `outcome`, `roll_id`
(new but not sensitive), and — once a recalc has happened — `recalc_history` (so they can see a
roll was corrected, though not necessarily reconstruct the discarded natural faces if `spec`/`raw`
inside older `RecalcEntry` values are separately GM-gated by the same rule, applied uniformly to
every `RollSpec`/`RawRoll`-shaped value anywhere under a `RollEmbed`, not just the top-level ones).

Client mirror: `chat-docs`'s Zod schema for `RollEmbed` gains matching optional `specHint`-free
fields only where the client actually renders something — the client needs `recalcHistory` (to
render a "recalculated" badge with a diff-style prior/after summary) and, for the GM's own view
only, enough of `raw`'s `DieRecord` list to power the recalc-op picker (§5). The client never
needs to parse a full `RollSpec` AST — the recalc UI works against `raw.records`/`raw.group_spans`
directly, which is already the shape `dice::recalc::recalculate` consumes for `ReplaceDie`/
`RemoveDice`, and `RerollDice` needs only a `DieId`, also read off `raw.records`.

## 5. Recalc operation & UI

- New WS intent: `RecalcRoll { message_id: Uuid, roll_id: Uuid, ops: Vec<RecalcOp> }`.
- Handler: GM check → load message doc → locate the `RollEmbed` by `roll_id` inside `content`
  (fail `RecalcRollError::RollNotFound` if absent — content order is not assumed stable) → refuse
  `RecalcRollError::NoStoredState` if `spec`/`raw` are `None` (a roll embedded before this feature
  shipped) → call `dice::recalc::recalculate(spec, raw, ops, rng)` → push a new `RecalcEntry`
  (capturing the pre-recalc `raw`/`outcome` as that entry's `previous_raw`/`previous_outcome`) onto
  `recalc_history`, then overwrite the live `raw`/`outcome` with `recalculate`'s output → publish via
  the chokepoint in §3.
- Client UI: a GM-only context menu on a `RollEmbed`'s rendered dice ("Reroll this die", "Remove
  this die", "Replace this die's face") — the closest existing precedent for a roll-scoped GM
  affordance is the composer's "Speak as" picker's GM-widened option set, not a new UI pattern.
  Non-GM recipients render a small "recalculated" indicator when `recalc_history` is non-empty,
  with no interactive affordance.

## 6. Anti-cheat posture

The system never silently discards a roll's original result: `recalc_history` is append-only, and
every `RecalcEntry` retains the pre-recalc `raw`/`outcome` it replaced. A GM can therefore always
be held to account for what a recalc changed, and a full recalc-audit reconstruction (were `spec`/
`raw` fully wire-exposed to GMs, which they are per §4) is always possible. This is a strictly
stronger anti-cheat posture than today's alternative (a GM manually editing chat, which the
existing `RollImmutable` check already forbids) — recalc is a controlled, audited replacement for
a capability GMs need in practice (correcting a misconfigured formula, honoring a house-rule
reroll) but that the current immutable-roll design has no sanctioned path for at all.

## 7. Testing

- `dice::recalc` itself needs no new tests (already covered).
- `chat::rolls`: `execute_roll_with_seed` now returns `spec`/`raw` alongside `formula`/`outcome` —
  update its existing call sites/tests to assert the new fields are populated.
- New `handle_recalc_roll` tests: GM can recalc a `Public`/`Whisper`/`GmOnly` roll regardless of
  audience (mirrors existing edit/delete audience-independence tests); non-GM sender is rejected
  even for their own roll; `RollNotFound`/`NoStoredState` paths; `recalc_history` accumulates
  correctly across two sequential recalcs on the same roll; the pre-existing-document case (a
  `RollEmbed` with `spec: None` seeded directly into a test fixture) is refused.
- New redaction tests: a non-GM recipient's filtered view of a `RollEmbed` never contains `spec`/
  `raw` (including inside `recalc_history` entries), a GM's view contains both, mirroring the
  `documents-permissions` skill's existing `gm_only` field coverage pattern — this is the one part
  of this feature that touches the shared redaction classifier, so it gets the heavier
  buddy-check-caliber review tier this campaign already established for permission-adjacent work
  in Phase 1b.

## 8. Non-goals

- No player self-service recalc (§3).
- No change to how `RollButton` (re-roll-the-whole-formula-fresh) works — that is an unrelated,
  already-shipped, unrestricted "roll again from scratch" affordance with no continuity promise to
  the original roll, and stays exactly as is.
- No retroactive backfill of `spec`/`raw` for rolls embedded before this feature ships — those
  rolls simply cannot be recalculated (§5's `NoStoredState` refusal), which is the correct, honest
  behavior rather than fabricating a plausible-looking history.
