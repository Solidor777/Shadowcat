# Recalc-From-Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist a roll's parsed `RollSpec`/natural-face `RawRoll` on `RollEmbed` and wire a GM-only, audited "recalculate this roll" affordance from chat through to `dice::recalc::recalculate`, replacing a roll's outcome with a visibly-marked, append-only-logged correction.

**Architecture:** `chat::rolls::execute_roll_with_seed` starts keeping the `RollSpec`/`RawRoll` it already builds instead of discarding them, threading them onto `Segment::RollEmbed`. A new GM-only `RecalcRoll` WS intent reuses the existing `WriteOrigin::ServerMessageRevision` chokepoint (the same one `handle_edit_message`/`handle_delete_message` use) to locate the targeted roll by a new stable `roll_id`, re-derive it, and append a `RecalcEntry` audit record. `spec`/`raw` (and each history entry's `previous_raw`) are GM-visible only, via the SAME per-document `permissions.property_overrides` mechanism every other GM-only field already uses — not a bespoke filter.

**Tech Stack:** Rust (server: `dice`, `chat`, `data::sqlite`, `ws::protocol`, `ws::conn`), TypeScript/Zod (`@shadowcat/core`'s wire mirror), Svelte 5 (`@shadowcat/module-chat-card`), ts-rs (WS frame codegen).

**Spec:** `docs/superpowers/specs/2026-08-21-recalc-from-chat-design.md` — every design fork it resolves (GM-only authorization, `property_overrides`-based wire exposure, append-only mutation semantics, anti-cheat posture) is FINAL; this plan is a mechanical decomposition of that spec, not a re-design.

## Global Constraints

- No player self-service recalc — GM-only, audience-independent (spec §3).
- `dice::recalc` itself is unmodified (already correct and tested) — this plan never edits `dice::eval::*`/`dice::recalc::recalculate`'s own logic.
- `dice` remains a pure library: no `#[derive(TS)]`/ts-rs bindings on any `dice::*` type, ever (crate-level hard invariant, `src/server/src/dice/mod.rs`'s doc comment).
- No retroactive backfill of `spec`/`raw` for pre-existing rolls — a `RollEmbed` with `spec: None`/`raw: None` simply cannot be recalculated (`RecalcRollError::NoStoredState`).
- `recalc_history`/`roll_id` are NOT GM-only; `spec`/`raw` (top-level AND every `RecalcEntry.previous_raw`) ARE GM-only, via `permissions.property_overrides`, never a chat-specific filter.
- `#![deny(missing_docs)]` + `#![deny(clippy::missing_docs_in_private_items)]` cover every touched Rust module (`chat`, `dice`, `data`, `ws`) — every new `pub`/private item needs a doc comment.
- No lint suppressions (`#[allow(...)]`, `#[expect(...)]`) without explicit user sign-off — none are needed by this plan.
- `cargo test --all` runs from the repo root (`C:\Dev\Shadowcat`); `pnpm --filter <pkg> test`/`pnpm --filter <pkg> test:types` run from the repo root too (pnpm workspace filters resolve regardless of cwd).
- ts-rs bindings regenerate via `cargo test --all` (writes `src/types/generated/*.ts`) — regenerate and commit alongside any `#[derive(TS)]` type change.

---

## Spec-vs-codebase note (read before Task 3)

The design spec's §4 states `data::validation::redaction_target` "gains a new recognized shape" for `spec`/`raw`. This is not how the mechanism actually works: `redaction_target` (which actually lives in `data::permission`, not `data::validation`) is a generic, field-name-agnostic classifier over `property_overrides` MAP KEYS — it already handles any pointer under the `engine` band with no changes needed. **Task 2 does not modify `redaction_target`/`filter_properties`/`collect_hidden` at all.** What the spec's intent actually requires is that the message-CONSTRUCTING code populate `doc.permissions.property_overrides` with `gm_only` entries naming each `spec`/`raw`/`previous_raw` pointer — that population is what Task 2 (initial send) and Task 3 (recalc) implement.

Task 3 also surfaces a genuine spec/codebase conflict: appending a new `RecalcEntry` requires ADDING a `property_overrides` entry on every recalc, which means writing `/permissions/property_overrides` — a path `WriteOrigin::ServerMessageRevision`'s existing scoped `Access` grant explicitly and deliberately excludes (`all: false`, "authorizes writing `/engine` only, never `/permissions`/`/embedded`, even for this trusted origin" — a documented Hard Invariant in `shadowcat-codebase-chat`). Granting that origin `cap::EDIT_PERMISSIONS` broadly would ALSO authorize rewriting `permissions.default`/`gm_role`/`users` — the message's own audience-enforcement fields — which is a materially bigger widening than the feature needs. Task 3 resolves this with the narrowest available mechanism: an exact-path admission (`ch.path == "/permissions/property_overrides"`, nothing else) colocated with the SAME already-bespoke `MESSAGE_DOC_TYPE`+`ServerMessageRevision` branch in `apply_intent`, touching no capability, no `required_cap_for_path` behavior for any other doc_type/path, and no other `ServerMessageRevision` caller. This is flagged here, in the code comments Task 3 adds, and in the dispatching report — it is a genuine widening of a documented, hardened security chokepoint and warrants human awareness before merge, even though it does not block this plan (the campaign directive's "determine the best long-term shape" question is answerable, as reasoned above).

---

### Task 1: `RollEmbed`/`RecalcEntry` data model + thread `spec`/`raw` through `execute_roll_with_seed`

**Files:**
- Modify: `src/server/src/dice/recalc.rs` (`RecalcOp` derive)
- Modify: `src/server/src/chat/mod.rs` (`Segment::RollEmbed`, new `RecalcEntry` struct, imports)
- Modify: `src/server/src/chat/rolls.rs` (`execute_roll_with_seed`/`execute_roll` signatures + doc comments + existing test call sites)
- Test: `src/server/src/chat/rolls.rs` (`#[cfg(test)] mod tests`, in-file)
- Test: `src/server/src/chat/mod.rs` (`#[cfg(test)] mod tests`, in-file)

**Interfaces:**
- Consumes: `crate::dice::{RollSpec, RawRoll, RecalcOp, roll, evaluate}` (`src/server/src/dice/mod.rs`'s existing re-exports), `crate::dice::rng::NoiseRng`, `crate::dice::notation`.
- Produces: `Segment::RollEmbed { formula: String, outcome: RollOutcome, roll_id: Uuid, spec: Option<RollSpec>, raw: Option<RawRoll>, recalc_history: Option<Vec<RecalcEntry>> }`; `chat::RecalcEntry { ops: Vec<RecalcOp>, previous_raw: RawRoll, previous_outcome: RollOutcome, recalculated_by: Uuid, recalculated_at: i64 }`; `chat::rolls::execute_roll`/`execute_roll_with_seed(..) -> Result<(String, RollOutcome, RollSpec, RawRoll), RollError>`. Task 2/3 consume `RecalcEntry` and the new `RollEmbed` fields directly.

- [ ] **Step 1: Add `Serialize`/`Deserialize` to `RecalcOp`**

`RecalcOp` will be embedded (via `RecalcEntry.ops`) inside the stored `engine` JSON body, so it needs serde derives (NOT `TS` — `dice` never gets ts-rs bindings; the wire-frame mirror is a separate type added in Task 3).

In `src/server/src/dice/recalc.rs`, change:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecalcOp {
```

to:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecalcOp {
```

and add the import at the top of the file (after the existing `use crate::dice::spec::{DieId, DieKind, Expr, RollSpec};` line):

```rust
use serde::{Deserialize, Serialize};
```

- [ ] **Step 2: Run the dice crate's tests to confirm the derive compiles cleanly**

Run: `cargo test --all -p shadowcat dice::recalc`
Expected: existing `dice::recalc` tests still PASS (no behavior change, only new derives).

- [ ] **Step 3: Write the new `RollEmbed` shape and `RecalcEntry` struct as a failing test**

In `src/server/src/chat/mod.rs`, inside `#[cfg(test)] mod tests` (the block starting near the file's existing `mod tests { use super::*; ... }`), add:

```rust
    #[test]
    fn roll_embed_carries_roll_id_spec_raw_and_defaults_recalc_history_to_none() {
        let spec = crate::dice::notation::parse(
            "1d6",
            crate::dice::ParseContext {
                mode: crate::dice::notation::ModeKind::Total,
                direction: crate::dice::spec::Direction::HighWins,
            },
        )
        .unwrap();
        let mut rng = crate::dice::rng::NoiseRng::from_seed(1);
        let raw = crate::dice::roll(&spec, &mut rng);
        let outcome = crate::dice::evaluate(&spec, &raw);
        let seg = Segment::RollEmbed {
            formula: "1d6".into(),
            outcome,
            roll_id: Uuid::from_u128(1),
            spec: Some(spec.clone()),
            raw: Some(raw.clone()),
            recalc_history: None,
        };
        let j = serde_json::to_value(&seg).unwrap();
        assert_eq!(j["roll_id"], serde_json::json!(Uuid::from_u128(1)));
        assert!(j.get("spec").is_some(), "spec must serialize when Some");
        assert!(j.get("raw").is_some(), "raw must serialize when Some");
        assert!(
            j.get("recalc_history").is_none(),
            "recalc_history: None must not serialize (skip_serializing_if)"
        );
        let back: Segment = serde_json::from_value(j).unwrap();
        assert_eq!(back, seg);
    }

    #[test]
    fn roll_embed_without_roll_id_deserializes_with_a_fresh_generated_one() {
        // A roll embedded before this field existed has no `roll_id` key at all —
        // `#[serde(default = "Uuid::new_v4")]` fills one in rather than failing to parse.
        let old_json = serde_json::json!({
            "kind": "roll_embed",
            "formula": "1d6",
            "outcome": {
                "total": 3, "records": [], "successes": null, "pass": null, "margin": null,
                "tier_label": null, "tier_value": null, "crit_successes": 0, "crit_fails": 0,
                "positive_counter": 0, "negative_counter": 0, "symbol_counts": {}, "labeled_consts": []
            }
        });
        let seg: Segment = serde_json::from_value(old_json).unwrap();
        match seg {
            Segment::RollEmbed { roll_id, spec, raw, recalc_history, .. } => {
                assert_ne!(roll_id, Uuid::nil());
                assert!(spec.is_none(), "a pre-existing roll has no stored spec");
                assert!(raw.is_none(), "a pre-existing roll has no stored raw");
                assert!(recalc_history.is_none());
            }
            other => panic!("expected RollEmbed, got {other:?}"),
        }
    }

    #[test]
    fn recalc_entry_round_trips() {
        let spec = crate::dice::notation::parse(
            "1d6",
            crate::dice::ParseContext {
                mode: crate::dice::notation::ModeKind::Total,
                direction: crate::dice::spec::Direction::HighWins,
            },
        )
        .unwrap();
        let mut rng = crate::dice::rng::NoiseRng::from_seed(2);
        let raw = crate::dice::roll(&spec, &mut rng);
        let outcome = crate::dice::evaluate(&spec, &raw);
        let entry = RecalcEntry {
            ops: vec![crate::dice::RecalcOp::RerollDice(vec![0])],
            previous_raw: raw,
            previous_outcome: outcome,
            recalculated_by: Uuid::from_u128(9),
            recalculated_at: 1000,
        };
        let j = serde_json::to_value(&entry).unwrap();
        let back: RecalcEntry = serde_json::from_value(j).unwrap();
        assert_eq!(back, entry);
    }
```

- [ ] **Step 4: Run the new tests to confirm they fail to compile (the types don't exist yet)**

Run: `cargo test --all -p shadowcat chat::tests::roll_embed_carries_roll_id`
Expected: FAIL to compile — `Segment::RollEmbed` has no `roll_id`/`spec`/`raw`/`recalc_history` fields, `RecalcEntry` is undefined.

- [ ] **Step 5: Add `RecalcEntry` and extend `Segment::RollEmbed`**

In `src/server/src/chat/mod.rs`, add the new imports alongside the existing `use crate::dice::RollOutcome;` line (around line 58):

```rust
use crate::dice::{RawRoll, RecalcOp, RollOutcome, RollSpec};
```

Replace the existing `RollEmbed` variant (currently):

```rust
    RollEmbed {
        /// The formula as the author wrote it.
        formula: String,
        /// The full deterministic outcome, natural faces included.
        outcome: RollOutcome,
    },
```

with:

```rust
    RollEmbed {
        /// The formula as the author wrote it.
        formula: String,
        /// The full deterministic outcome, natural faces included. Overwritten by
        /// `handle_recalc_roll` on each recalculation; the PRE-recalc value is
        /// preserved as the newest `recalc_history` entry's `previous_outcome`.
        outcome: RollOutcome,
        /// Stable identity for this roll, independent of its position in `content`
        /// -- a recalc targets a roll by this id, never by array index, so it
        /// survives any future reordering (e.g. link-preview enrichment appending
        /// later segments). Defaults to a fresh id on deserialize so a roll
        /// embedded before this field existed still round-trips.
        #[serde(default = "Uuid::new_v4")]
        roll_id: Uuid,
        /// The parsed formula this roll was scored from, kept so a GM can later
        /// recalculate it. `None` for any roll embedded before this field existed
        /// -- `handle_recalc_roll` refuses `NoStoredState` on `None`, never
        /// guesses a spec back from `outcome`. GM-visible only (see
        /// `roll_embed_property_overrides`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spec: Option<RollSpec>,
        /// The natural-face roll log `outcome` was evaluated from, kept for the
        /// same recalculation purpose as `spec` (same None-for-pre-existing rule).
        /// GM-visible only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<RawRoll>,
        /// Present iff this roll has been recalculated at least once: an ordered,
        /// append-only audit log, each entry retaining the PRE-recalc
        /// `raw`/`outcome` it replaced -- the roll's original result is never
        /// silently discarded. Visible to every recipient (unlike `spec`/`raw`);
        /// each entry's OWN `previous_raw` is separately GM-gated.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recalc_history: Option<Vec<RecalcEntry>>,
    },
```

Also update the doc comment directly above the variant (currently starting `/// A completed roll: the formula plus its full deterministic outcome. ... The RollSpec/RawRoll are deliberately NOT stored -- recalculate-from-chat is out of scope pre-release ...`), replacing the last two sentences of that comment block with:

```rust
    /// A completed roll: the formula plus its full deterministic outcome.
    /// `outcome` embeds the evaluated `RollOutcome` (records included -- the
    /// natural faces make the roll reproducible/auditable from the stored
    /// segment alone). `spec`/`raw` are kept (not discarded) so a GM can later
    /// recalculate this roll via `handle_recalc_roll`; `recalc_history` records
    /// every such recalculation. Produced only by `chat::rolls::execute_roll`,
    /// called from `handle_send_message`'s roll stage; a fresh embed is never
    /// produced on edit (rolls are immutable, see `handle_edit_message`) --
    /// `handle_recalc_roll` is the only path that ever mutates an existing one.
```

Now add `RecalcEntry` immediately after the `Segment` enum's closing `}` (right before the `plain_text_content` function):

```rust
/// One applied recalculation of a `RollEmbed`, appended to its `recalc_history`.
/// `previous_raw`/`previous_outcome` are the PRE-recalc state this entry
/// replaced -- the roll's live `raw`/`outcome` after the Nth entry is the Nth
/// entry's OUTPUT, which is either the (N+1)th entry's
/// `previous_raw`/`previous_outcome` or, for the last entry, the current
/// `RollEmbed.raw`/`RollEmbed.outcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecalcEntry {
    /// The targeted mutation(s) applied this recalculation.
    pub ops: Vec<RecalcOp>,
    /// The roll's natural-face log immediately BEFORE this recalculation.
    /// GM-visible only, same as `Segment::RollEmbed`'s `raw` field.
    pub previous_raw: RawRoll,
    /// The roll's outcome immediately BEFORE this recalculation. Visible to
    /// every recipient (not GM-gated) -- same visibility as `RollEmbed::outcome`.
    pub previous_outcome: RollOutcome,
    /// The GM who performed this recalculation.
    pub recalculated_by: Uuid,
    /// Epoch milliseconds this recalculation was applied -- this codebase's
    /// timestamp convention throughout `MessageEngine` (`edited_at`/`deleted_at`)
    /// and `Document` (`created_at`/`updated_at`) is an epoch-millisecond `i64`,
    /// never `chrono`, which is not a dependency anywhere in this crate; this
    /// field follows that established sibling-field convention rather than
    /// introducing a new dependency for one field.
    pub recalculated_at: i64,
}
```

- [ ] **Step 6: Run the new tests to confirm they pass**

Run: `cargo test --all -p shadowcat chat::tests::roll_embed_carries_roll_id chat::tests::roll_embed_without_roll_id chat::tests::recalc_entry_round_trips`
Expected: PASS.

- [ ] **Step 7: Thread `spec`/`raw` through `execute_roll_with_seed`/`execute_roll`**

In `src/server/src/chat/rolls.rs`, change the function signature and body (currently):

```rust
fn execute_roll_with_seed(
    formula: &str,
    ctx: ParseContext,
    seed: u64,
) -> Result<(String, RollOutcome), RollError> {
    let spec = notation::parse(formula, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)?;
    let mut rng = NoiseRng::from_seed(seed);
    let raws = roll(&spec, &mut rng);
    if raws.records.len() > MAX_ROLL_RECORDS {
        return Err(RollError::TooManyRecords(raws.records.len()));
    }
    let outcome = eval::evaluate(&spec, &raws);
    Ok((formula.to_owned(), outcome))
}
```

to:

```rust
fn execute_roll_with_seed(
    formula: &str,
    ctx: ParseContext,
    seed: u64,
) -> Result<(String, RollOutcome, RollSpec, crate::dice::RawRoll), RollError> {
    let spec = notation::parse(formula, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)?;
    let mut rng = NoiseRng::from_seed(seed);
    let raws = roll(&spec, &mut rng);
    if raws.records.len() > MAX_ROLL_RECORDS {
        return Err(RollError::TooManyRecords(raws.records.len()));
    }
    let outcome = eval::evaluate(&spec, &raws);
    Ok((formula.to_owned(), outcome, spec, raws))
}
```

and the passthrough wrapper (currently):

```rust
pub(crate) fn execute_roll(
    formula: &str,
    ctx: ParseContext,
) -> Result<(String, RollOutcome), RollError> {
    execute_roll_with_seed(formula, ctx, entropy_seed())
}
```

to:

```rust
pub(crate) fn execute_roll(
    formula: &str,
    ctx: ParseContext,
) -> Result<(String, RollOutcome, RollSpec, crate::dice::RawRoll), RollError> {
    execute_roll_with_seed(formula, ctx, entropy_seed())
}
```

Also update the doc comment directly above `execute_roll` (currently ending "...The ONLY untrusted-notation execution path in chat. Seeds from `entropy_seed()` -- fresh OS entropy per call, never a caller-supplied or persisted seed.") by appending one sentence:

```rust
/// Also returns the parsed `RollSpec` and rolled `RawRoll` alongside the
/// formula/outcome, so a caller can persist them onto `Segment::RollEmbed`
/// for a later GM recalculation.
```

- [ ] **Step 8: Update `execute_roll_with_seed`'s own doc comment for the new return arity**

The doc comment directly above `execute_roll_with_seed` currently reads "Test seam: identical to `execute_roll` but takes an explicit seed instead of drawing fresh OS entropy, so a test can assert on a deterministic outcome. `execute_roll` is a thin entropy-supplying wrapper over this." -- leave this sentence as-is (still accurate); no change needed here.

- [ ] **Step 9: Update the two call sites in `handle_send_message`**

In `src/server/src/chat/mod.rs`, change (the top-level `/roll` branch, currently):

```rust
        match rolls::execute_roll(&parsed.body, dice_ctx) {
            Ok((formula, outcome)) => vec![Segment::RollEmbed { formula, outcome }],
```

to:

```rust
        match rolls::execute_roll(&parsed.body, dice_ctx) {
            Ok((formula, outcome, spec, raw)) => vec![Segment::RollEmbed {
                formula,
                outcome,
                roll_id: Uuid::new_v4(),
                spec: Some(spec),
                raw: Some(raw),
                recalc_history: None,
            }],
```

and (the inline `[[...]]` roll branch, currently):

```rust
                        match rolls::execute_roll(formula, dice_ctx.unwrap()) {
                            Ok((formula, outcome)) => {
                                segments.push(Segment::RollEmbed { formula, outcome })
                            }
```

to:

```rust
                        match rolls::execute_roll(formula, dice_ctx.unwrap()) {
                            Ok((formula, outcome, spec, raw)) => {
                                segments.push(Segment::RollEmbed {
                                    formula,
                                    outcome,
                                    roll_id: Uuid::new_v4(),
                                    spec: Some(spec),
                                    raw: Some(raw),
                                    recalc_history: None,
                                })
                            }
```

- [ ] **Step 10: Update `execute_roll_with_seed`'s existing test call sites in `rolls.rs`**

In `src/server/src/chat/rolls.rs`'s `#[cfg(test)] mod tests`, update the two tuple-destructuring tests. Change:

```rust
    #[test]
    fn execute_roll_returns_the_formula_verbatim() {
        let (formula, _) = execute_roll_with_seed("2d6+1", total_ctx(), 1).unwrap();
        assert_eq!(formula, "2d6+1");
    }
```

to:

```rust
    #[test]
    fn execute_roll_returns_the_formula_verbatim() {
        let (formula, _, _, _) = execute_roll_with_seed("2d6+1", total_ctx(), 1).unwrap();
        assert_eq!(formula, "2d6+1");
    }
```

Change:

```rust
        let (_, out) = execute_roll_with_seed("2000000000*2000000000*3", total_ctx(), 1).unwrap();
        assert_eq!(out.total, i64::MAX);
```

to:

```rust
        let (_, out, _, _) = execute_roll_with_seed("2000000000*2000000000*3", total_ctx(), 1).unwrap();
        assert_eq!(out.total, i64::MAX);
```

Change:

```rust
        let (_, out) = execute_roll_with_seed("1d10000*1d10000", total_ctx(), 7).unwrap();
        assert!(out.total >= 0);
```

to:

```rust
        let (_, out, _, _) = execute_roll_with_seed("1d10000*1d10000", total_ctx(), 7).unwrap();
        assert!(out.total >= 0);
```

`execute_roll_with_seed_is_deterministic` (`let a = ...; let b = ...; assert_eq!(a, b);`) needs no change -- it compares the whole tuple, which now also asserts `spec`/`raw` equality, strictly stronger than before.

- [ ] **Step 11: Add a test asserting `spec`/`raw` are populated**

Add to `src/server/src/chat/rolls.rs`'s test module:

```rust
    #[test]
    fn execute_roll_with_seed_returns_spec_and_raw_matching_the_outcome() {
        let (_, outcome, spec, raw) = execute_roll_with_seed("2d6+1", total_ctx(), 5).unwrap();
        // spec/raw are exactly what `evaluate` was run against -- re-evaluating
        // them independently must reproduce the same outcome.
        assert_eq!(crate::dice::evaluate(&spec, &raw), outcome);
        assert_eq!(raw.dice.len(), 2);
    }
```

- [ ] **Step 12: Run the full server test suite**

Run: `cargo test --all`
Expected: PASS (no other call sites of `execute_roll`/`execute_roll_with_seed` exist outside `chat/mod.rs` and `chat/rolls.rs`).

- [ ] **Step 13: Commit**

```bash
git add src/server/src/dice/recalc.rs src/server/src/chat/mod.rs src/server/src/chat/rolls.rs
git commit -m "feat(chat): persist RollSpec/RawRoll on RollEmbed, add RecalcEntry"
```

---

### Task 2: GM-only wire exposure for `spec`/`raw` via `property_overrides`

**Files:**
- Modify: `src/server/src/chat/mod.rs` (new `roll_embed_property_overrides` helper, `build_message_doc` wiring)
- Test: `src/server/src/chat/mod.rs` (`#[cfg(test)] mod tests`, in-file)

**Interfaces:**
- Consumes: `Segment::RollEmbed`/`RecalcEntry` (Task 1); `crate::data::document::Visibility`, `crate::data::permission::redaction_target` (existing, unmodified), `crate::data::permission::{filter_properties, Access}` (existing, unmodified, exercised by this task's tests only).
- Produces: `pub(crate) fn roll_embed_property_overrides(content: &[Segment]) -> BTreeMap<String, Visibility>` (consumed by Task 3's `handle_recalc_roll`).

- [ ] **Step 1: Write a failing test for the overrides helper**

Add to `src/server/src/chat/mod.rs`'s test module:

```rust
    #[test]
    fn roll_embed_property_overrides_marks_spec_raw_and_recalc_history_previous_raw_gm_only() {
        use crate::data::document::Visibility;

        let spec = crate::dice::notation::parse(
            "1d6",
            crate::dice::ParseContext {
                mode: crate::dice::notation::ModeKind::Total,
                direction: crate::dice::spec::Direction::HighWins,
            },
        )
        .unwrap();
        let mut rng = crate::dice::rng::NoiseRng::from_seed(3);
        let raw = crate::dice::roll(&spec, &mut rng);
        let outcome = crate::dice::evaluate(&spec, &raw);

        let content = vec![
            Segment::Text { text: "before ".into() },
            Segment::RollEmbed {
                formula: "1d6".into(),
                outcome: outcome.clone(),
                roll_id: Uuid::from_u128(1),
                spec: Some(spec.clone()),
                raw: Some(raw.clone()),
                recalc_history: Some(vec![RecalcEntry {
                    ops: vec![crate::dice::RecalcOp::RerollDice(vec![0])],
                    previous_raw: raw.clone(),
                    previous_outcome: outcome,
                    recalculated_by: Uuid::from_u128(2),
                    recalculated_at: 100,
                }]),
            },
        ];

        let overrides = roll_embed_property_overrides(&content);
        assert_eq!(
            overrides.get("/engine/content/1/spec"),
            Some(&Visibility::GmOnly)
        );
        assert_eq!(
            overrides.get("/engine/content/1/raw"),
            Some(&Visibility::GmOnly)
        );
        assert_eq!(
            overrides.get("/engine/content/1/recalc_history/0/previous_raw"),
            Some(&Visibility::GmOnly)
        );
        assert_eq!(
            overrides.get("/engine/content/1/recalc_history/0/previous_outcome"),
            None,
            "previous_outcome is visible to every recipient, never gm_only"
        );
        assert_eq!(overrides.len(), 3, "no other pointers should be marked");
    }

    #[test]
    fn roll_embed_property_overrides_is_empty_for_non_roll_content() {
        let content = vec![Segment::Text { text: "hi".into() }];
        assert!(roll_embed_property_overrides(&content).is_empty());
    }

    #[test]
    fn roll_embed_property_overrides_skips_a_pre_existing_roll_with_no_spec_raw() {
        // A roll embedded before this feature shipped: spec/raw are None, so no
        // override entries should be produced for it (nothing to hide).
        let outcome = crate::dice::evaluate(
            &crate::dice::notation::parse(
                "1d6",
                crate::dice::ParseContext {
                    mode: crate::dice::notation::ModeKind::Total,
                    direction: crate::dice::spec::Direction::HighWins,
                },
            )
            .unwrap(),
            &crate::dice::roll(
                &crate::dice::notation::parse(
                    "1d6",
                    crate::dice::ParseContext {
                        mode: crate::dice::notation::ModeKind::Total,
                        direction: crate::dice::spec::Direction::HighWins,
                    },
                )
                .unwrap(),
                &mut crate::dice::rng::NoiseRng::from_seed(4),
            ),
        );
        let content = vec![Segment::RollEmbed {
            formula: "1d6".into(),
            outcome,
            roll_id: Uuid::from_u128(5),
            spec: None,
            raw: None,
            recalc_history: None,
        }];
        assert!(roll_embed_property_overrides(&content).is_empty());
    }
```

- [ ] **Step 2: Run the tests to confirm they fail to compile**

Run: `cargo test --all -p shadowcat chat::tests::roll_embed_property_overrides`
Expected: FAIL to compile -- `roll_embed_property_overrides` is undefined.

- [ ] **Step 3: Implement `roll_embed_property_overrides`**

In `src/server/src/chat/mod.rs`, add the import (extend the existing `use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};` line):

```rust
use crate::data::document::{DocRole, Document, PermissionSet, Scope, Visibility, WorldRole};
```

Add the helper function right after `RecalcEntry`'s closing `}` (before `plain_text_content`):

```rust
/// Computes the `gm_only` `permissions.property_overrides` entries a
/// message's roll content requires: `spec`/`raw` on every `RollEmbed`, plus
/// `previous_raw` on every one of its `recalc_history` entries. Applied
/// uniformly to every `RollSpec`/`RawRoll`-shaped value under a `RollEmbed`
/// -- `outcome`/`previous_outcome`/`recalc_history` itself stay visible to
/// every recipient. Recomputed from scratch against the CURRENT `content`
/// (never incrementally patched), so a message's override set always matches
/// what it actually carries; called from `build_message_doc` at Create time
/// and from `handle_recalc_roll` after every recalculation.
pub(crate) fn roll_embed_property_overrides(
    content: &[Segment],
) -> BTreeMap<String, Visibility> {
    let mut out = BTreeMap::new();
    for (i, seg) in content.iter().enumerate() {
        let Segment::RollEmbed {
            spec,
            raw,
            recalc_history,
            ..
        } = seg
        else {
            continue;
        };
        if spec.is_some() {
            out.insert(format!("/engine/content/{i}/spec"), Visibility::GmOnly);
        }
        if raw.is_some() {
            out.insert(format!("/engine/content/{i}/raw"), Visibility::GmOnly);
        }
        if let Some(history) = recalc_history {
            for j in 0..history.len() {
                out.insert(
                    format!("/engine/content/{i}/recalc_history/{j}/previous_raw"),
                    Visibility::GmOnly,
                );
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `cargo test --all -p shadowcat chat::tests::roll_embed_property_overrides`
Expected: PASS.

- [ ] **Step 5: Wire the helper into `build_message_doc`**

In `src/server/src/chat/mod.rs`, `build_message_doc` currently constructs `PermissionSet` as:

```rust
        permissions: PermissionSet {
            default,
            users,
            gm_role,
            ..Default::default()
        },
```

Change to:

```rust
        permissions: PermissionSet {
            default,
            users,
            gm_role,
            property_overrides: roll_embed_property_overrides(&engine.content),
            ..Default::default()
        },
```

- [ ] **Step 6: Write a redaction-regression integration test**

Add to `src/server/src/chat/mod.rs`'s test module (this exercises the REAL redaction path -- `data::permission::{resolve_access, filter_properties}` -- against a `build_message_doc` output, so it fails if the wiring above is wrong):

```rust
    #[tokio::test]
    async fn a_roll_messages_spec_and_raw_are_gm_only_but_outcome_and_roll_id_are_not() {
        use crate::data::document::WorldRole as DocWorldRole;
        use crate::data::permission::{filter_properties, resolve_access};
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, DocWorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: DocWorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = crate::ws::PingRateLimiter::new();

        let cmd = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &PermissionContext {
                    user_id: player,
                    world_role: DocWorldRole::Player,
                },
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            "/roll 1d6".into(),
            None,
            Audience::Public,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc.clone(),
            other => panic!("expected Create, got {other:?}"),
        };

        // Non-GM player: spec/raw are nulled; outcome/roll_id survive.
        let player_access = resolve_access(player, DocWorldRole::Player, &doc, Some(player));
        let player_view = filter_properties(&doc, &player_access).unwrap();
        let sys: serde_json::Value = player_view.engine.clone().unwrap();
        let seg = &sys["content"][0];
        assert_eq!(seg["spec"], serde_json::Value::Null);
        assert_eq!(seg["raw"], serde_json::Value::Null);
        assert!(seg.get("outcome").is_some() && !seg["outcome"].is_null());
        assert!(seg.get("roll_id").is_some() && !seg["roll_id"].is_null());

        // GM: spec/raw survive unredacted.
        let gm_access = resolve_access(gm, DocWorldRole::Gm, &doc, Some(player));
        let gm_view = filter_properties(&doc, &gm_access).unwrap();
        let gm_sys: serde_json::Value = gm_view.engine.clone().unwrap();
        let gm_seg = &gm_sys["content"][0];
        assert!(gm_seg.get("spec").is_some() && !gm_seg["spec"].is_null());
        assert!(gm_seg.get("raw").is_some() && !gm_seg["raw"].is_null());
    }
```

Note: this test's exact `resolve_access` signature/argument order must match the current `data::permission::resolve_access` function -- read `src/server/src/data/permission.rs`'s `resolve_access` signature before writing this step and adjust the call if it differs from `resolve_access(user, world_role, doc, effective_owner)`.

- [ ] **Step 7: Run the test**

Run: `cargo test --all -p shadowcat chat::tests::a_roll_messages_spec_and_raw_are_gm_only`
Expected: PASS.

- [ ] **Step 8: Run the full server test suite**

Run: `cargo test --all`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat): gate RollEmbed spec/raw as gm_only via property_overrides"
```

---

### Task 3: `RecalcRoll` WS intent + `handle_recalc_roll` handler

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent`'s exact-path `/permissions/property_overrides` admission)
- Modify: `src/server/src/chat/mod.rs` (`WireRecalcOp`, `RecalcRollError`, `handle_recalc_roll`, module doc-comment update)
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::RecalcRoll` variant)
- Modify: `src/server/src/ws/conn.rs` (new dispatch arm)
- Test: `src/server/src/data/sqlite.rs` (`#[cfg(test)] mod tests`, in-file)
- Test: `src/server/src/chat/mod.rs` (`#[cfg(test)] mod tests`, in-file)
- Test: `src/server/src/ws/protocol.rs` (`#[cfg(test)] mod tests`, in-file)
- Generated (regenerate, do not hand-edit): `src/types/generated/ClientMsg.ts`, `src/types/generated/WireRecalcOp.ts`

**Interfaces:**
- Consumes: `Segment::RollEmbed`, `RecalcEntry`, `roll_embed_property_overrides` (Tasks 1-2); `crate::dice::{recalculate, RecalcOp, DieId}`; `crate::data::permission::{cap, resolve_access, filter_properties}` (existing, unmodified); `MessageRequestCtx`-sibling positional-args pattern from `handle_delete_message`.
- Produces: `chat::WireRecalcOp` (`#[derive(TS)]`, ts-rs-exported), `chat::RecalcRollError`, `chat::handle_recalc_roll(room, repo, ctx, rate, message_id, roll_id, ops, now, budget_per_min) -> Result<Command, RecalcRollError>`, `ws::protocol::ClientMsg::RecalcRoll { request_id, message_id, roll_id, ops }`. Task 4 consumes `WireRecalcOp`'s generated TS shape and `ClientMsg`'s `"recalc_roll"` tag.

- [ ] **Step 1: Write a failing test for the `apply_intent` exact-path admission**

Add to `src/server/src/data/sqlite.rs`'s test module (near the other `apply_intent_*` tests):

```rust
    #[tokio::test]
    async fn apply_intent_server_message_revision_may_write_property_overrides_but_nothing_else_under_permissions() {
        use crate::chat::MESSAGE_DOC_TYPE;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
        use crate::data::document::{DocRole, PermissionSet, Scope};
        use crate::data::membership::PermissionContext;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext {
            user_id: gm,
            world_role: crate::data::document::WorldRole::Gm,
        };

        let doc_id = uuid::Uuid::new_v4();
        let doc = crate::data::document::Document {
            id: doc_id,
            scope: Scope::World { world_id: w.id },
            doc_type: MESSAGE_DOC_TYPE.to_string(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: Some(gm),
            permissions: PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            embedded: Default::default(),
            parent_id: None,
            engine: Some(serde_json::json!({
                "channel": "all", "user_owner": gm, "kind": "normal",
                "audience": {"kind": "public"}, "content": []
            })),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        };
        repo.apply_intent(
            w.id,
            &ctx,
            vec![Operation::Create { doc: doc.clone() }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // `/permissions/property_overrides` is admitted under ServerMessageRevision.
        let ok = repo
            .apply_intent(
                w.id,
                &ctx,
                vec![Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/permissions/property_overrides".into(),
                        old: serde_json::json!({}),
                        new: serde_json::json!({"/engine/content/0/spec": "gm_only"}),
                    }],
                }],
                1,
                WriteOrigin::ServerMessageRevision,
            )
            .await;
        assert!(ok.is_ok(), "property_overrides write should be admitted: {ok:?}");

        // `/permissions/default` is NOT admitted under the same origin.
        let denied = repo
            .apply_intent(
                w.id,
                &ctx,
                vec![Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/permissions/default".into(),
                        old: serde_json::json!("observer"),
                        new: serde_json::json!("owner"),
                    }],
                }],
                2,
                WriteOrigin::ServerMessageRevision,
            )
            .await;
        assert!(
            matches!(denied, Err(DataError::Forbidden)),
            "widening /permissions/default must stay forbidden under ServerMessageRevision, got {denied:?}"
        );
    }
```

Note: read the exact current signature of `Repository::apply_intent`/`SqliteRepository::apply_intent` and the exact `Document` field list (`embedded`'s type, `system`'s type) in `src/server/src/data/sqlite.rs`/`src/server/src/data/document.rs` before writing this step, and adjust the constructed values/call shape to match exactly (this plan's earlier research read `Document.permissions`/`Document.engine`/`Document.system` field names and types but the implementer should re-verify the constructor call compiles against the current struct).

- [ ] **Step 2: Run the test to confirm it fails**

Run: `cargo test --all -p shadowcat apply_intent_server_message_revision_may_write_property_overrides`
Expected: FAIL -- the `/permissions/property_overrides` write is currently rejected (`DataError::Forbidden`), same as `/permissions/default`.

- [ ] **Step 3: Add the exact-path admission in `apply_intent`**

In `src/server/src/data/sqlite.rs`, inside `apply_intent`'s `Operation::Update` arm, the capability-check block currently reads:

```rust
                        let need = required_cap_for_path(&ch.path).ok_or(DataError::Forbidden)?;
                        if !access.has(need) {
                            tracing::debug!(
                                user = %ctx.user_id, path = %ch.path, capability = need,
                                "intent denied: missing capability"
                            );
                            return Err(DataError::Forbidden);
                        }
```

Change to:

```rust
                        let need = required_cap_for_path(&ch.path).ok_or(DataError::Forbidden)?;
                        if !access.has(need) {
                            // A `ServerMessageRevision` write to a message doc may
                            // ALSO write exactly `/permissions/property_overrides`
                            // (never any other `/permissions` subpath) without
                            // holding `cap::EDIT_PERMISSIONS` -- `handle_recalc_roll`
                            // needs this to register a freshly-appended
                            // `RecalcEntry`'s gm_only override pointer. Granting
                            // `EDIT_PERMISSIONS` to this origin instead would ALSO
                            // authorize rewriting `default`/`gm_role`/`users` -- the
                            // message's own audience-enforcement fields -- which
                            // this origin's `all: false` scoping deliberately
                            // excludes (see the `ServerMessageRevision` access-grant
                            // construction above). This exact-path admission widens
                            // nothing for any other doc_type/origin/path.
                            let is_recalc_override_write = cur.doc_type
                                == crate::chat::MESSAGE_DOC_TYPE
                                && origin == WriteOrigin::ServerMessageRevision
                                && ch.path == "/permissions/property_overrides";
                            if !is_recalc_override_write {
                                tracing::debug!(
                                    user = %ctx.user_id, path = %ch.path, capability = need,
                                    "intent denied: missing capability"
                                );
                                return Err(DataError::Forbidden);
                            }
                        }
```

- [ ] **Step 4: Run the test to confirm it passes**

Run: `cargo test --all -p shadowcat apply_intent_server_message_revision_may_write_property_overrides`
Expected: PASS.

- [ ] **Step 5: Update `chat/mod.rs`'s module-level doc comment**

The module doc comment (top of `src/server/src/chat/mod.rs`) currently ends: "...exempting ONLY `WriteOrigin::ServerMessageRevision` -- a marker no wire frame can set, produced solely by `handle_edit_message`/`handle_delete_message` after their own owner-or-GM check -- and granting it a scoped `Access` (`READ`+`WRITE_FIELDS` only, never `/permissions`/`/embedded`)."

Change the final clause to:

```rust
//! ... and granting it a scoped `Access` (`READ`+`WRITE_FIELDS` only, plus an
//! exact-path admission in `data::sqlite::apply_intent` for
//! `/permissions/property_overrides` on a message doc under this origin --
//! see `handle_recalc_roll` -- never any other `/permissions` subpath, and
//! never `/embedded`). `handle_recalc_roll` is this origin's third producer,
//! after its own GM-only check (never owner-or-GM -- see
//! `RecalcRollError::Forbidden`).
```

- [ ] **Step 6: Write a failing test for `WireRecalcOp`'s conversion**

Add to `src/server/src/chat/mod.rs`'s test module:

```rust
    #[test]
    fn wire_recalc_op_converts_to_dice_recalc_op() {
        assert_eq!(
            WireRecalcOp::RerollDice { ids: vec![1, 2] }.into_recalc_op(),
            crate::dice::RecalcOp::RerollDice(vec![1, 2])
        );
        assert_eq!(
            WireRecalcOp::ReplaceDie { id: 3, natural: 5 }.into_recalc_op(),
            crate::dice::RecalcOp::ReplaceDie { id: 3, natural: 5 }
        );
        assert_eq!(
            WireRecalcOp::RemoveDice { ids: vec![4] }.into_recalc_op(),
            crate::dice::RecalcOp::RemoveDice(vec![4])
        );
    }
```

- [ ] **Step 7: Run the test to confirm it fails to compile**

Run: `cargo test --all -p shadowcat chat::tests::wire_recalc_op_converts`
Expected: FAIL to compile -- `WireRecalcOp` is undefined.

- [ ] **Step 8: Add `WireRecalcOp`**

In `src/server/src/chat/mod.rs`, add near the existing `ActorOwnerRef`/`Audience` types (both `pub`, `#[derive(TS)]`, near the top of the file after the `MessageKind`/before `Segment`, matching where those two live today):

```rust
/// Client-facing mirror of `dice::RecalcOp`, carried on the `RecalcRoll` wire
/// frame. `dice` is a pure library with no ts-rs bindings by design (see
/// `dice`'s crate doc) -- this type exists solely so a `RecalcOp` can ride
/// `ClientMsg`, converted via `into_recalc_op` before it ever reaches
/// `dice::recalc::recalculate`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireRecalcOp {
    /// Draw a fresh natural for each targeted die.
    RerollDice {
        /// Targeted die ids.
        ids: Vec<u32>,
    },
    /// Force a specific natural onto one die.
    ReplaceDie {
        /// The targeted die.
        id: u32,
        /// The natural face to force.
        natural: i32,
    },
    /// Drop targeted dice from their group's base naturals entirely.
    RemoveDice {
        /// Targeted die ids.
        ids: Vec<u32>,
    },
}

impl WireRecalcOp {
    /// Converts the wire shape into the dice engine's own `RecalcOp`.
    pub(crate) fn into_recalc_op(self) -> RecalcOp {
        match self {
            WireRecalcOp::RerollDice { ids } => RecalcOp::RerollDice(ids),
            WireRecalcOp::ReplaceDie { id, natural } => RecalcOp::ReplaceDie { id, natural },
            WireRecalcOp::RemoveDice { ids } => RecalcOp::RemoveDice(ids),
        }
    }
}
```

- [ ] **Step 9: Run the test to confirm it passes**

Run: `cargo test --all -p shadowcat chat::tests::wire_recalc_op_converts`
Expected: PASS.

- [ ] **Step 10: Add `RecalcRollError`**

In `src/server/src/chat/mod.rs`, add directly after `SendMessageError`'s `Display` impl closing `}` (before the `MessageRequestCtx` struct):

```rust
/// Why `handle_recalc_roll` refused a `RecalcRoll` frame.
#[derive(Debug)]
pub enum RecalcRollError {
    /// The requester holds no GM role in this world. Recalc is GM-only,
    /// audience-independent -- never owner-or-GM (see the module's design
    /// note on why there is no player self-service tier).
    Forbidden,
    /// The target message does not exist, or is not a `message` doc.
    NotFound,
    /// No `RollEmbed` in the message's content carries the given `roll_id`.
    RollNotFound,
    /// The targeted `RollEmbed` has no stored `spec`/`raw` -- it was embedded
    /// before this feature shipped and cannot be recalculated.
    NoStoredState,
    /// The user's per-minute flood budget is exhausted.
    RateLimited,
    /// The authoritative write failed.
    Data(DataError),
}

/// Player-presentable text for a `RecalcRoll` rejection (correlated
/// `ChatError`). `[sec]`-classified like `SendMessageError::Display`:
/// `Forbidden`/`NotFound`/`RollNotFound` collapse to one generic string (no
/// existence oracle); `NoStoredState` is safe to state exactly (recalc is
/// GM-only, so only an already-authorized GM ever sees it); `Data` never
/// leaks its inner detail.
impl std::fmt::Display for RecalcRollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecalcRollError::Forbidden
            | RecalcRollError::NotFound
            | RecalcRollError::RollNotFound => {
                f.write_str("You are not permitted to modify this message.")
            }
            RecalcRollError::NoStoredState => {
                f.write_str("This roll has no stored state to recalculate.")
            }
            RecalcRollError::RateLimited => {
                f.write_str("You are sending messages too quickly. Please wait a moment.")
            }
            RecalcRollError::Data(_) => {
                f.write_str("The message could not be delivered. Please try again.")
            }
        }
    }
}
```

- [ ] **Step 11: Write failing tests for `handle_recalc_roll`**

Add to `src/server/src/chat/mod.rs`'s test module:

```rust
    async fn seed_gm_and_room() -> (
        crate::data::sqlite::SqliteRepository,
        crate::ws::room::Room,
        Uuid,
        Uuid,
        Uuid,
    ) {
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        (repo, room, w.id, gm, player)
    }

    #[tokio::test]
    async fn handle_recalc_roll_rejects_a_non_gm_sender() {
        let (repo, room, _world, gm, player) = seed_gm_and_room().await;
        let rate = PingRateLimiter::new();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let cmd = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &gm_ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            "/roll 1d6".into(),
            None,
            Audience::Public,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc.clone(),
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
        let roll_id = match &sys.content[0] {
            Segment::RollEmbed { roll_id, .. } => *roll_id,
            other => panic!("expected RollEmbed, got {other:?}"),
        };

        let player_ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let err = handle_recalc_roll(
            &room,
            &repo,
            &player_ctx,
            &rate,
            doc.id,
            roll_id,
            vec![],
            101,
            30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RecalcRollError::Forbidden));
    }

    #[tokio::test]
    async fn handle_recalc_roll_rejects_unknown_roll_id_and_missing_stored_state() {
        let (repo, room, _world, gm, _player) = seed_gm_and_room().await;
        let rate = PingRateLimiter::new();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let cmd = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &gm_ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            "/roll 1d6".into(),
            None,
            Audience::Public,
        )
        .await
        .unwrap();
        let message_id = match &cmd.ops[0] {
            Operation::Create { doc } => doc.id,
            other => panic!("expected Create, got {other:?}"),
        };

        let err = handle_recalc_roll(
            &room,
            &repo,
            &gm_ctx,
            &rate,
            message_id,
            Uuid::from_u128(999_999),
            vec![],
            101,
            30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RecalcRollError::RollNotFound));
    }

    #[tokio::test]
    async fn handle_recalc_roll_refuses_a_roll_with_no_stored_spec_or_raw() {
        // A `RollEmbed` seeded directly with `spec: None`/`raw: None` -- the
        // pre-existing-document case (a roll embedded before this feature
        // shipped). Seeded by hand-crafting the stored `engine` JSON rather
        // than going through `handle_send_message` (which always populates
        // both), since that is the only way to construct this legacy shape.
        let (repo, room, world, gm, _player) = seed_gm_and_room().await;
        let rate = PingRateLimiter::new();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let roll_id = Uuid::from_u128(42);
        let outcome = crate::dice::evaluate(
            &crate::dice::notation::parse(
                "1d6",
                crate::dice::ParseContext {
                    mode: crate::dice::notation::ModeKind::Total,
                    direction: crate::dice::spec::Direction::HighWins,
                },
            )
            .unwrap(),
            &crate::dice::roll(
                &crate::dice::notation::parse(
                    "1d6",
                    crate::dice::ParseContext {
                        mode: crate::dice::notation::ModeKind::Total,
                        direction: crate::dice::spec::Direction::HighWins,
                    },
                )
                .unwrap(),
                &mut crate::dice::rng::NoiseRng::from_seed(11),
            ),
        );
        let content = vec![Segment::RollEmbed {
            formula: "1d6".into(),
            outcome,
            roll_id,
            spec: None,
            raw: None,
            recalc_history: None,
        }];
        let doc = build_message_doc(
            world,
            gm,
            MessageDraft {
                channel: "all".into(),
                actor_owner: None,
                audience: Audience::Public,
                kind: MessageKind::Roll,
                content,
                source: None,
            },
            0,
        );
        repo.apply_intent(
            world,
            &gm_ctx,
            vec![Operation::Create { doc: doc.clone() }],
            0,
            crate::data::command::WriteOrigin::Client,
        )
        .await
        .unwrap();

        let err = handle_recalc_roll(&room, &repo, &gm_ctx, &rate, doc.id, roll_id, vec![], 101, 30)
            .await
            .unwrap_err();
        assert!(matches!(err, RecalcRollError::NoStoredState));
    }

    #[tokio::test]
    async fn handle_recalc_roll_succeeds_for_public_whisper_and_gmonly_audiences() {
        // Audience-independence (mirrors handle_edit_message/handle_delete_message's
        // own audience-independence tests): a GM's moderation authority to recalc
        // is the same regardless of who can otherwise READ the message.
        for audience in [
            Audience::Public,
            Audience::Whisper {
                recipients: vec![],
            },
            Audience::GmOnly,
        ] {
            let (repo, room, _world, gm, _player) = seed_gm_and_room().await;
            let rate = PingRateLimiter::new();
            let gm_ctx = PermissionContext {
                user_id: gm,
                world_role: WorldRole::Gm,
            };
            let cmd = handle_send_message(
                MessageRequestCtx {
                    room: &room,
                    repo: &repo,
                    ctx: &gm_ctx,
                    rate: &rate,
                    preview: LinkPreviewDeps {
                        client: &link_preview::build_client_allow_loopback(),
                        cache: &LinkPreviewCache::new(),
                        rate: &PreviewRateLimiter::new(),
                    },
                    now: 100,
                    budget_per_min: 30,
                },
                "all".into(),
                "/roll 1d6".into(),
                None,
                audience.clone(),
            )
            .await
            .unwrap();
            let doc = match &cmd.ops[0] {
                Operation::Create { doc } => doc.clone(),
                other => panic!("expected Create, got {other:?}"),
            };
            let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
            let roll_id = match &sys.content[0] {
                Segment::RollEmbed { roll_id, .. } => *roll_id,
                other => panic!("expected RollEmbed, got {other:?}"),
            };
            let ok = handle_recalc_roll(&room, &repo, &gm_ctx, &rate, doc.id, roll_id, vec![], 101, 30)
                .await;
            assert!(
                ok.is_ok(),
                "recalc must succeed under {audience:?}, got {ok:?}"
            );
        }
    }

    #[tokio::test]
    async fn handle_recalc_roll_applies_a_reroll_and_appends_recalc_history() {
        let (repo, room, _world, gm, player) = seed_gm_and_room().await;
        let rate = PingRateLimiter::new();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let cmd = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &gm_ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 30,
            },
            "gmonly".into(),
            "/roll 1d6".into(),
            None,
            Audience::GmOnly,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc.clone(),
            other => panic!("expected Create, got {other:?}"),
        };
        let before: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        let (roll_id, before_raw, before_outcome) = match &before.content[0] {
            Segment::RollEmbed {
                roll_id, raw, outcome, ..
            } => (*roll_id, raw.clone().unwrap(), outcome.clone()),
            other => panic!("expected RollEmbed, got {other:?}"),
        };
        let target_id = before_raw.dice[0].id;

        let cmd2 = handle_recalc_roll(
            &room,
            &repo,
            &gm_ctx,
            &rate,
            doc.id,
            roll_id,
            vec![WireRecalcOp::RerollDice {
                ids: vec![target_id],
            }
            .into_recalc_op()],
            101,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd2.ops.len(), 1);
        let changes = match &cmd2.ops[0] {
            Operation::Update { changes, .. } => changes,
            other => panic!("expected Update, got {other:?}"),
        };
        assert!(changes.iter().any(|c| c.path == "/engine"));
        assert!(changes
            .iter()
            .any(|c| c.path == "/permissions/property_overrides"));

        let stored = repo.get_document(doc.id).await.unwrap().unwrap();
        let after: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
        match &after.content[0] {
            Segment::RollEmbed {
                recalc_history, ..
            } => {
                let history = recalc_history.as_ref().unwrap();
                assert_eq!(history.len(), 1);
                assert_eq!(history[0].previous_raw, before_raw);
                assert_eq!(history[0].previous_outcome, before_outcome);
                assert_eq!(history[0].recalculated_by, gm);
            }
            other => panic!("expected RollEmbed, got {other:?}"),
        }

        // A second recalc by a GM not individually listed on this GmOnly message
        // still succeeds (moderation authority is audience-independent) and
        // accumulates a SECOND history entry.
        let cmd3 = handle_recalc_roll(&room, &repo, &gm_ctx, &rate, doc.id, roll_id, vec![], 102, 30)
            .await
            .unwrap();
        assert_eq!(cmd3.ops.len(), 1);
        let stored2 = repo.get_document(doc.id).await.unwrap().unwrap();
        let after2: MessageEngine = serde_json::from_value(stored2.engine.unwrap()).unwrap();
        match &after2.content[0] {
            Segment::RollEmbed { recalc_history, .. } => {
                assert_eq!(recalc_history.as_ref().unwrap().len(), 2);
            }
            other => panic!("expected RollEmbed, got {other:?}"),
        }

        // Redaction check: a non-GM's filtered view of the message, AFTER two
        // recalcs, still never contains spec/raw at the top level OR inside
        // EITHER recalc_history entry's previous_raw -- while
        // previous_outcome/recalc_history itself stay visible.
        let player_access = crate::data::permission::resolve_access(
            player,
            WorldRole::Player,
            &stored2,
            Some(gm),
        );
        let player_view = crate::data::permission::filter_properties(&stored2, &player_access).unwrap();
        let player_sys: serde_json::Value = player_view.engine.unwrap();
        let seg = &player_sys["content"][0];
        assert_eq!(seg["spec"], serde_json::Value::Null);
        assert_eq!(seg["raw"], serde_json::Value::Null);
        let history = seg["recalc_history"].as_array().unwrap();
        assert_eq!(history.len(), 2, "recalc_history itself is visible to a non-GM");
        for entry in history {
            assert_eq!(
                entry["previous_raw"],
                serde_json::Value::Null,
                "every recalc_history entry's previous_raw must stay gm_only"
            );
            assert!(
                entry.get("previous_outcome").is_some() && !entry["previous_outcome"].is_null(),
                "previous_outcome must stay visible to a non-GM"
            );
        }
    }
```

Note: this test's exact `resolve_access`/`filter_properties` call shape must match the current `data::permission` signatures -- see Task 2 Step 6's identical note.

- [ ] **Step 12: Run the tests to confirm they fail to compile**

Run: `cargo test --all -p shadowcat chat::tests::handle_recalc_roll`
Expected: FAIL to compile -- `handle_recalc_roll` is undefined.

- [ ] **Step 13: Implement `handle_recalc_roll`**

In `src/server/src/chat/mod.rs`, add directly after `handle_delete_message`'s closing `}` (before `#[cfg(test)] mod tests`):

```rust
/// Server-authoritative roll correction: GM-only (never owner-or-GM -- see
/// `RecalcRollError::Forbidden`), locates the targeted `RollEmbed` by
/// `roll_id` (never by array index), re-derives it via `dice::recalculate`,
/// and appends an auditable `RecalcEntry` capturing the PRE-recalc
/// `raw`/`outcome` before overwriting them. Reuses
/// `WriteOrigin::ServerMessageRevision` -- the SAME chokepoint
/// `handle_edit_message`/`handle_delete_message` use -- as its third caller.
/// Also writes `/permissions/property_overrides` in the SAME
/// `Operation::Update`, which `apply_intent`'s `ServerMessageRevision` branch
/// admits ONLY at that exact path (see `data::sqlite::apply_intent`'s
/// exact-path admission) -- needed because a freshly-appended
/// `RecalcEntry.previous_raw` pointer must be added to the GM-only override
/// set on every recalc.
pub async fn handle_recalc_roll(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    message_id: Uuid,
    roll_id: Uuid,
    ops: Vec<RecalcOp>,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, RecalcRollError> {
    if ctx.world_role != WorldRole::Gm {
        return Err(RecalcRollError::Forbidden);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(RecalcRollError::RateLimited);
    }
    let cur = repo
        .get_document(message_id)
        .await
        .map_err(RecalcRollError::Data)?
        .ok_or(RecalcRollError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return Err(RecalcRollError::NotFound);
    }
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap_or_default())
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;

    let idx = sys
        .content
        .iter()
        .position(|seg| matches!(seg, Segment::RollEmbed { roll_id: rid, .. } if *rid == roll_id))
        .ok_or(RecalcRollError::RollNotFound)?;

    // First pass: read immutably and clone what's needed, so the mutation
    // below never overlaps a live borrow.
    let (spec_val, raw_val, prev_outcome) = match &sys.content[idx] {
        Segment::RollEmbed {
            spec: Some(s),
            raw: Some(r),
            outcome,
            ..
        } => (s.clone(), r.clone(), outcome.clone()),
        Segment::RollEmbed { .. } => return Err(RecalcRollError::NoStoredState),
        _ => unreachable!("idx matched only Segment::RollEmbed above"),
    };

    let seed = rolls::entropy_seed();
    let mut rng = NoiseRng::from_seed(seed);
    let (new_raw, new_outcome) = crate::dice::recalculate(&spec_val, &raw_val, &ops, &mut rng);

    let entry = RecalcEntry {
        ops,
        previous_raw: raw_val,
        previous_outcome: prev_outcome,
        recalculated_by: ctx.user_id,
        recalculated_at: now,
    };
    if let Segment::RollEmbed {
        raw,
        outcome,
        recalc_history,
        ..
    } = &mut sys.content[idx]
    {
        recalc_history.get_or_insert_with(Vec::new).push(entry);
        *raw = Some(new_raw);
        *outcome = new_outcome;
    }

    let overrides = roll_embed_property_overrides(&sys.content);
    let new_engine = serde_json::to_value(&sys)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;
    let new_overrides_json = serde_json::to_value(&overrides)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;
    let old_overrides_json = serde_json::to_value(&cur.permissions.property_overrides)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![
            FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur.engine.clone().unwrap_or_default(),
                new: new_engine,
            },
            FieldChange {
                remove: false,
                path: "/permissions/property_overrides".into(),
                old: old_overrides_json,
                new: new_overrides_json,
            },
        ],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(RecalcRollError::Data)
}
```

Add the new import at the top of the file (extend the existing `use crate::dice::{RawRoll, RecalcOp, RollOutcome, RollSpec};` line to also bring in `NoiseRng`):

```rust
use crate::dice::rng::NoiseRng;
use crate::dice::{RawRoll, RecalcOp, RollOutcome, RollSpec};
```

- [ ] **Step 14: Run the tests to confirm they pass**

Run: `cargo test --all -p shadowcat chat::tests::handle_recalc_roll`
Expected: PASS.

- [ ] **Step 15: Add `ClientMsg::RecalcRoll`**

In `src/server/src/ws/protocol.rs`, extend the import:

```rust
use crate::chat::{ActorOwnerRef, Audience, WireRecalcOp};
```

Add a new variant to `ClientMsg`, directly after the `DeleteMessage` variant's closing `},` (still inside the enum, before the enum's own closing `}`):

```rust
    /// GM-only roll correction: locates the targeted `RollEmbed` by `roll_id`
    /// (never by array index) and re-derives it via the dice engine's
    /// `recalculate`, appending an auditable `recalc_history` entry. Same
    /// asymmetric reply protocol as `SendMessage`; a non-GM sender is
    /// rejected via a correlated `ChatError`.
    RecalcRoll {
        /// Correlation token for a `ChatError` rejection.
        request_id: Uuid,
        /// The message carrying the targeted roll.
        message_id: Uuid,
        /// The targeted roll's stable id (`Segment::RollEmbed::roll_id`).
        roll_id: Uuid,
        /// The targeted mutation(s) to apply.
        ops: Vec<WireRecalcOp>,
    },
```

- [ ] **Step 16: Write a wire-shape test for the new frame**

Add to `src/server/src/ws/protocol.rs`'s test module (near `edit_and_delete_frames_carry_request_id`):

```rust
    #[test]
    fn recalc_roll_frame_parses() {
        let raw = r#"{"type":"recalc_roll","request_id":"00000000-0000-0000-0000-0000000000ad","message_id":"00000000-0000-0000-0000-000000000001","roll_id":"00000000-0000-0000-0000-000000000002","ops":[{"kind":"reroll_dice","ids":[1,2]},{"kind":"replace_die","id":3,"natural":6},{"kind":"remove_dice","ids":[4]}]}"#;
        let msg: ClientMsg = serde_json::from_str(raw).unwrap();
        match msg {
            ClientMsg::RecalcRoll {
                request_id,
                message_id,
                roll_id,
                ops,
            } => {
                assert_eq!(request_id, Uuid::from_u128(0xad));
                assert_eq!(message_id, Uuid::from_u128(1));
                assert_eq!(roll_id, Uuid::from_u128(2));
                assert_eq!(ops.len(), 3);
                assert!(matches!(
                    ops[0],
                    crate::chat::WireRecalcOp::RerollDice { .. }
                ));
                assert!(matches!(
                    ops[1],
                    crate::chat::WireRecalcOp::ReplaceDie {
                        id: 3,
                        natural: 6
                    }
                ));
                assert!(matches!(
                    ops[2],
                    crate::chat::WireRecalcOp::RemoveDice { .. }
                ));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
```

- [ ] **Step 17: Run `cargo test --all` to regenerate ts-rs bindings and confirm the new tests pass**

Run: `cargo test --all`
Expected: PASS; `src/types/generated/ClientMsg.ts` and `src/types/generated/WireRecalcOp.ts` are written/updated on disk.

- [ ] **Step 18: Inspect and stage the regenerated bindings**

Run: `git status src/types/generated`
Expected: `ClientMsg.ts` modified (new `recalc_roll` arm), `WireRecalcOp.ts` newly created.

- [ ] **Step 19: Add the `ws::conn.rs` dispatch arm**

In `src/server/src/ws/conn.rs`, directly after the `ClientMsg::DeleteMessage { .. }` arm's closing `}` (before the `ClientMsg::Pathfind { .. }` arm), add:

```rust
                                Ok(ClientMsg::RecalcRoll {
                                    request_id,
                                    message_id,
                                    roll_id,
                                    ops,
                                }) => {
                                    // Same confirm-by-broadcast-echo shape as
                                    // SendMessage/EditMessage/DeleteMessage; a
                                    // rejection is surfaced to the sender only via a
                                    // correlated `ChatError`.
                                    if let Err(e) = crate::chat::handle_recalc_roll(
                                        &room,
                                        repo.as_ref(),
                                        &ctx,
                                        &message_rate,
                                        message_id,
                                        roll_id,
                                        ops.into_iter()
                                            .map(crate::chat::WireRecalcOp::into_recalc_op)
                                            .collect(),
                                        now_millis(),
                                        MESSAGE_RATE_PER_MIN,
                                    )
                                    .await
                                    {
                                        tracing::debug!(world = %world_id, user = %user_id, ?e, "recalc rejected");
                                        if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                            request_id,
                                            message: e.to_string(),
                                        }))).await.is_err() {
                                            break;
                                        }
                                    }
                                }
```

Note: read the exact current text of the `ClientMsg::DeleteMessage { .. }` arm (whose call site was verified earlier in this plan's research to use `&room, repo.as_ref(), &ctx, &message_rate, message_id, now_millis(), MESSAGE_RATE_PER_MIN`) immediately before this edit to confirm variable names (`room`/`repo`/`ctx`/`message_rate`/`etx`/`world_id`/`user_id`) match this connection's local scope exactly -- copy their exact spelling from that arm, not from this plan's prose.

- [ ] **Step 20: Run the full server test suite**

Run: `cargo test --all`
Expected: PASS.

- [ ] **Step 21: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS (no warnings).

- [ ] **Step 22: Commit**

```bash
git add src/server/src/data/sqlite.rs src/server/src/chat/mod.rs src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated
git commit -m "feat(chat): add RecalcRoll WS intent + handle_recalc_roll handler"
```

---

### Task 4: Client wire mirror + session/`ChatApi` plumbing

**Files:**
- Modify: `src/client/core/src/chat-docs.ts` (`ChatSegment`'s `roll_embed` arm, new `WireDieKind`/`WireRawRoll`/`RecalcHistoryEntry` types+schemas, `baseRollDice`/`numericBounds` helpers)
- Modify: `src/client/core/src/wire.ts` (`WireRecalcOp` type, `ClientMsg`'s `recalc_roll` variant)
- Modify: `src/client/core/src/ws-client.ts` (`recalcRoll` method)
- Modify: `src/client/core/src/index.ts` (new exports)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (`recalcRoll` method)
- Modify: `src/client/shell/src/lib/Table.svelte` (`chat.recalc` wiring)
- Modify: `src/client/ui-kit/src/appContext.ts` (`ChatApi.recalc`)
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts` (default `chat.recalc`)
- Modify: `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte` (`chat.recalc`)
- Modify: `src/modules/chat-card/src/MessageCard.test.ts` (4 existing `chat: {...}` literals gain `recalc`)
- Test: `src/client/core/src/chat-docs.test.ts`
- Test: `src/client/core/src/wire.test.ts`
- Test: `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Consumes: `Ts.ClientMsg`/`Ts.WireRecalcOp` (generated from Task 3's `src/types/generated/`).
- Produces: `WireRecalcOp` (TS union), `ChatSegment`'s `roll_embed` arm gains `roll_id?: string`, `spec?: unknown`, `raw?: WireRawRoll | null`, `recalc_history?: RecalcHistoryEntry[] | null`; `baseRollDice(raw: WireRawRoll)`, `numericBounds(kind: WireDieKind)`; `WsClient.recalcRoll(messageId, rollId, ops)`; `ChatApi.recalc(messageId, rollId, ops)`. Task 5 consumes `baseRollDice`/`numericBounds`/`ChatApi.recalc`/the new `ChatSegment` fields directly.

- [ ] **Step 1: Write failing drift-guard + parse tests for the new `chat-docs.ts` types**

Add to `src/client/core/src/chat-docs.test.ts`, inside the existing `describe("roll segments", ...)` block:

```ts
  test("parses roll_id, GM-visible spec/raw, and recalc_history when present", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d6", outcome: rollOutcome(),
        roll_id: "11111111-1111-1111-1111-111111111111",
        raw: {
          dice: [{ id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 4 }],
          group_spans: [[0, 1]],
        },
        recalc_history: [{
          previous_outcome: rollOutcome({ total: 2 }),
          recalculated_by: "u-gm",
          recalculated_at: 100,
        }],
      }],
    }));
    expect(eng).not.toBeNull();
    const seg = eng!.content[0] as Extract<typeof eng.content[number], { kind: "roll_embed" }>;
    expect(seg.roll_id).toBe("11111111-1111-1111-1111-111111111111");
    expect(seg.raw?.dice[0].natural).toBe(4);
    expect(seg.recalc_history?.[0].recalculated_by).toBe("u-gm");
  });
  test("roll_embed without roll_id/spec/raw/recalc_history still parses (legacy/non-GM shape)", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{ kind: "roll_embed", formula: "1d6", outcome: rollOutcome() }],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([{ kind: "roll_embed", formula: "1d6", outcome: rollOutcome() }]);
  });
```

Add a new `describe` block (below `describe("roll segments", ...)`) for the pure helpers:

```ts
describe("baseRollDice / numericBounds", () => {
  test("baseRollDice slices raw.dice by each group_spans range, in order", () => {
    const raw = {
      dice: [
        { id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 3 },
        { id: 1, kind: { Numeric: { min: 1, max: 6 } }, natural: 5 },
        { id: 2, kind: { Numeric: { min: 1, max: 6 } }, natural: 6 }, // an explosion child past both spans
      ],
      group_spans: [[0, 2]] as [number, number][],
    };
    expect(baseRollDice(raw).map((d) => d.id)).toEqual([0, 1]);
  });
  test("numericBounds returns the range for a Numeric die, null for Faces", () => {
    expect(numericBounds({ Numeric: { min: 1, max: 20 } })).toEqual({ min: 1, max: 20 });
    expect(numericBounds({ Faces: { faces: [{ value: 1, symbols: [] }] } })).toBeNull();
  });
});
```

Update the imports at the top of `chat-docs.test.ts` to bring in the new symbols:

```ts
import {
  parseMessageEngine,
  buildChannelRegistryDoc,
  buildDiceSettingsDoc,
  buildChatSettingsDoc,
  isKnownSegment,
  MESSAGE_DOC_TYPE,
  dieRecordSchemaImpl,
  constTermSchemaImpl,
  chatSegmentSchemaImpl,
  rollOutcomeSchemaImpl,
  chatMessageEngineSchemaImpl,
  baseRollDice,
  numericBounds,
  type DieRecord,
  type ConstTerm,
  type ChatSegment,
  type RollOutcome,
  type ChatMessageEngine,
} from "./chat-docs";
```

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `pnpm --filter @shadowcat/core test -- chat-docs`
Expected: FAIL -- `baseRollDice`/`numericBounds` are undefined; the new fields fail to parse (currently rejected by the strict object schema, or simply absent from the output type).

- [ ] **Step 3: Add `WireDieKind`, `WireRawRoll`, `RecalcHistoryEntry`, and the two helpers to `chat-docs.ts`**

In `src/client/core/src/chat-docs.ts`, add (directly after `RollOutcomeSchema`'s declaration, before the `ChatSegment` type):

```ts
/** A die's face space for the recalc picker. Mirrors `dice::spec::DieKind`
 * (externally tagged -- the crate's plain serde default; `dice` carries no
 * ts-rs bindings by design). Only `Numeric` bounds are read by the client
 * (chat notation cannot produce `Faces` today); a `Faces` die still parses,
 * but `numericBounds` returns `null` for it and the "replace this die's
 * face" affordance simply does not render. */
export type WireDieKind =
  | { Numeric: { min: number; max: number } }
  | { Faces: { faces: { value?: number | null; symbols: string[] }[] } };

// Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
export const wireDieKindSchemaImpl = z.union([
  z.object({ Numeric: z.object({ min: z.number(), max: z.number() }) }),
  z.object({
    Faces: z.object({
      faces: z.array(z.object({ value: z.number().nullish(), symbols: z.array(z.string()) })),
    }),
  }),
]);
/** Validator for a `WireDieKind`. */
export const WireDieKindSchema: z.ZodType<WireDieKind> = wireDieKindSchemaImpl;

/** A roll's natural-face log, mirroring `dice::outcome::RawRoll` -- GM-visible
 * only (see `ChatSegment`'s `roll_embed.raw` doc). Only `dice`/`group_spans`
 * are modeled: `baseRollDice` targets a roll's BASE dice only (`group_spans`'
 * index ranges into `dice`, excluding explosion/penetrate children, matching
 * `dice::recalc::recalculate`'s own restriction), reading each base die's
 * stable `id` and pre-modifier `natural` face directly off `dice`; `records`/
 * `next_id` carry no information this client needs and are intentionally
 * unmirrored (tolerated via `.passthrough()`, never rejected). */
export type WireRawRoll = {
  dice: { id: number; kind: WireDieKind; natural: number }[];
  group_spans: [number, number][];
};

// Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
export const wireRawRollSchemaImpl = z.object({
  dice: z.array(z.object({ id: z.number(), kind: WireDieKindSchema, natural: z.number() })),
  group_spans: z.array(z.tuple([z.number(), z.number()])),
});
/** Validator for a `WireRawRoll`. `.passthrough()` tolerates server-only
 * fields (`records`, `next_id`) this mirror does not model. */
export const WireRawRollSchema: z.ZodType<WireRawRoll> = wireRawRollSchemaImpl.passthrough();

/** One applied recalculation, mirroring `chat::RecalcEntry`. Only
 * `previous_outcome`/`recalculated_by`/`recalculated_at` are modeled -- the
 * card's "recalculated" badge shows a prior/after summary from
 * `previous_outcome` vs. the segment's own current `outcome`; `ops`/
 * `previous_raw` carry no rendered information (`previous_raw` is GM-only
 * server-side, and the client never needs to reconstruct a past recalc's
 * exact die-level state) and pass through unvalidated. */
export type RecalcHistoryEntry = {
  previous_outcome: RollOutcome;
  recalculated_by: string;
  recalculated_at: number;
};

// Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
export const recalcHistoryEntrySchemaImpl = z.object({
  previous_outcome: RollOutcomeSchema,
  recalculated_by: z.string(),
  recalculated_at: z.number(),
});
/** Validator for a `RecalcHistoryEntry`. Input type is widened to `unknown`
 * because `previous_outcome: RollOutcomeSchema` inherits that schema's own
 * widened input (see `RollOutcomeSchema`'s doc). `.passthrough()` tolerates
 * `ops`/`previous_raw`, which this mirror does not model. */
export const RecalcHistoryEntrySchema: z.ZodType<RecalcHistoryEntry, z.ZodTypeDef, unknown> =
  recalcHistoryEntrySchemaImpl.passthrough();

/** The BASE dice a recalc may target: `raw.dice` sliced by each
 * `group_spans` range (see `WireRawRoll`'s doc -- explosion/penetrate
 * children fall outside every span and are never recalc-targetable).
 * @param raw The roll's stored natural-face log.
 * @returns Every base die, in roll order.
 * @example
 * ```ts
 * import { baseRollDice } from "@shadowcat/core";
 *
 * baseRollDice({ dice: [{ id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 3 }], group_spans: [[0, 1]] });
 * ```
 */
export function baseRollDice(raw: WireRawRoll): { id: number; kind: WireDieKind; natural: number }[] {
  const out: { id: number; kind: WireDieKind; natural: number }[] = [];
  for (const [start, count] of raw.group_spans) {
    for (let i = start; i < start + count; i++) {
      const d = raw.dice[i];
      if (d) out.push(d);
    }
  }
  return out;
}

/** Numeric bounds for a die's "replace this die's face" input, or `null` for
 * a `Faces` die (chat notation cannot produce one today; the affordance
 * simply does not render -- see `WireDieKind`'s doc).
 * @param kind The die's face space.
 * @returns `{min, max}` for a `Numeric` die, else `null`.
 * @example
 * ```ts
 * import { numericBounds } from "@shadowcat/core";
 *
 * numericBounds({ Numeric: { min: 1, max: 20 } }); // { min: 1, max: 20 }
 * ```
 */
export function numericBounds(kind: WireDieKind): { min: number; max: number } | null {
  return "Numeric" in kind ? kind.Numeric : null;
}
```

Then update `ChatSegment`'s `roll_embed` arm (currently):

```ts
  | {
      /** A completed roll: the formula plus its full deterministic outcome. */
      kind: "roll_embed";
      /** The formula as the author wrote it. */
      formula: string;
      /** The full deterministic outcome, natural faces included. */
      outcome: RollOutcome;
    }
```

to:

```ts
  | {
      /** A completed roll: the formula plus its full deterministic outcome. */
      kind: "roll_embed";
      /** The formula as the author wrote it. */
      formula: string;
      /** The full deterministic outcome, natural faces included. */
      outcome: RollOutcome;
      /** Stable identity for this roll; a recalc targets it by this id, never by
       * array index. Optional here (not `.default(...)`) purely to tolerate a
       * malformed/legacy test fixture omitting it -- the server always emits it. */
      roll_id?: string;
      /** GM-visible only: the parsed spec this roll was scored from. Absent for a
       * roll embedded before recalc-from-chat shipped, or when this recipient is
       * not a GM. Kept opaque (`unknown`) -- the client never parses a full
       * `RollSpec`; only `raw` powers the recalc picker. */
      spec?: unknown;
      /** GM-visible only: the natural-face log this roll was evaluated from.
       * Powers the GM recalc picker (`baseRollDice`/`numericBounds`); absent for
       * a pre-existing roll or a non-GM recipient. */
      raw?: WireRawRoll | null;
      /** Present iff this roll has been recalculated at least once; visible to
       * every recipient (unlike `spec`/`raw`). */
      recalc_history?: RecalcHistoryEntry[] | null;
    }
```

Update `chatSegmentSchemaImpl`'s `roll_embed` arm (currently):

```ts
  z.object({ kind: z.literal("roll_embed"), formula: z.string(), outcome: RollOutcomeSchema }),
```

to:

```ts
  z.object({
    kind: z.literal("roll_embed"),
    formula: z.string(),
    outcome: RollOutcomeSchema,
    roll_id: z.string().optional(),
    spec: z.unknown().optional(),
    raw: WireRawRollSchema.nullish(),
    recalc_history: z.array(RecalcHistoryEntrySchema).nullish(),
  }),
```

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `pnpm --filter @shadowcat/core test -- chat-docs`
Expected: PASS.

- [ ] **Step 5: Add `WireRecalcOp` and the `recalc_roll` `ClientMsg` variant in `wire.ts`**

In `src/client/core/src/wire.ts`, add directly above the `ClientMsg` type declaration:

```ts
/** Client-facing mirror of the dice engine's `RecalcOp`, carried on the
 * `recalc_roll` frame. Mirrors `ws::protocol::WireRecalcOp` exactly (a
 * discriminated union on `kind`). */
export type WireRecalcOp =
  | { kind: "reroll_dice"; ids: number[] }
  | { kind: "replace_die"; id: number; natural: number }
  | { kind: "remove_dice"; ids: number[] };
```

Add a new variant to `ClientMsg`, directly after the `delete_message` variant (before the union's terminal `;`):

```ts
  | {
      /** GM-only roll correction: locates the targeted `RollEmbed` by `roll_id`
       * (never by array index) and re-derives it via the dice engine's
       * `recalculate`, appending an auditable `recalc_history` entry. Same
       * asymmetric reply protocol as `send_message`. */
      type: "recalc_roll";
      /** Correlation token for a `chat_error` rejection. */
      request_id: string;
      /** The message carrying the targeted roll. */
      message_id: string;
      /** The targeted roll's stable id. */
      roll_id: string;
      /** The targeted mutation(s) to apply. */
      ops: WireRecalcOp[];
    };
```

- [ ] **Step 6: Write a drift-guard assertion**

The existing generic guard in `src/client/core/src/wire.test.ts`:

```ts
  it("ClientMsg type tags", () => {
    expectTypeOf<ClientMsg["type"]>().toEqualTypeOf<Ts.ClientMsg["type"]>();
  });
```

already covers this (it fails to typecheck if `"recalc_roll"` exists on only one side). No new test needed here; run the existing suite in the next step to confirm it still passes with the new tag added on both sides.

- [ ] **Step 7: Run the type-check test suite**

Run: `pnpm --filter @shadowcat/core test -- wire`
Expected: PASS.

- [ ] **Step 8: Add `WsClient.recalcRoll`**

In `src/client/core/src/ws-client.ts`, extend the import from `./wire`:

```ts
import {
  parseServerMsg,
  type ClientMsg,
  type WireWelcome,
  type WireCommand,
  type WireSearchHit,
  type WireActorOwnerRef,
  type WireAudience,
  type WireRecalcOp,
} from "./wire";
```

Add the method directly after `deleteChatMessage`'s closing `}`:

```ts
  /** GM-only roll correction. Resolves/rejects like `sendChatMessage`; the
   * server rejects a non-GM sender via a correlated `chat_error`.
   * @param messageId The message carrying the targeted roll.
   * @param rollId The targeted roll's stable id.
   * @param ops The targeted mutation(s) to apply.
   * @returns Resolves (void) once the recalc is accepted; rejects with the
   * server's player-presentable reason otherwise.
   * @example
   * ```ts
   * import { WsClient, webSocketConnect } from "@shadowcat/core";
   *
   * const client = new WsClient({
   *   connect: webSocketConnect("wss://example.test/ws"),
   *   world: "world-1",
   *   handlers: { onCommand: () => {} },
   * });
   * await client.recalcRoll("msg-1", "roll-1", [{ kind: "reroll_dice", ids: [0] }]);
   * ```
   */
  recalcRoll(messageId: string, rollId: string, ops: WireRecalcOp[]): Promise<void> {
    const request_id = crypto.randomUUID();
    const p = this.trackChatOp(request_id);
    this.send({ type: "recalc_roll", request_id, message_id: messageId, roll_id: rollId, ops });
    return p;
  }
```

- [ ] **Step 9: Write a test for `recalcRoll`**

Find the existing `editChatMessage`/`deleteChatMessage` tests in `src/client/core/src/ws-client.test.ts` and add a sibling test following the same mock-transport pattern used there:

```ts
  it("recalcRoll sends a recalc_roll frame and resolves after the chat-error window with no error", async () => {
    // Mirror the exact mock-transport + fake-timer setup this file's
    // sendChatMessage/editChatMessage tests already use immediately above --
    // read that setup verbatim before writing this test, since the transport
    // mock/fake-timer helper names are local to this file and must match
    // exactly (do not invent new ones).
    const client = makeTestClient(); // use this file's existing client-construction helper
    await client.start();
    const p = client.recalcRoll("msg-1", "roll-1", [{ kind: "remove_dice", ids: [2] }]);
    vi.advanceTimersByTime(CHAT_ERROR_WINDOW_MS_FOR_TEST);
    await expect(p).resolves.toBeUndefined();
  });
```

Note: this file already has a working pattern for `sendChatMessage`/`editChatMessage`/`deleteChatMessage` tests (mock transport capturing sent frames, fake timers advancing past the chat-error window). Read that pattern in full before writing this step and match its exact helper names/setup -- the snippet above names the SHAPE of the test, not literal helper identifiers, which must be copied from the file itself.

- [ ] **Step 10: Run the ws-client test suite**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: PASS.

- [ ] **Step 11: Export the new symbols from `index.ts`**

In `src/client/core/src/index.ts`, extend the `from "./wire"` type-export block (currently ending `...WireMoveStreamSample, WireMoveStreamVisionSample, } from "./wire";`) to also include `WireRecalcOp`:

```ts
export type {
  ServerMsg,
  ClientMsg,
  WireDocument,
  WireCommand,
  WireOperation,
  WireFieldChange,
  WireScope,
  WireCapabilityGrants,
  WireCapabilityRequirement,
  WireContractProvide,
  WireContractDeclaration,
  WireSearchHit,
  WireActorOwnerRef,
  WireAudience,
  WirePermissionSet,
  WireMoveStreamSample,
  WireMoveStreamVisionSample,
  WireRecalcOp,
} from "./wire";
```

Extend the `chat-docs` export lines (currently):

```ts
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, ChatSegmentSchema, ChatMessageEngineSchema, parseMessageEngine, isKnownSegment, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
export type { MessageKind, DieRecord, RollOutcome, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm } from "./chat-docs";
```

to:

```ts
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, ChatSegmentSchema, ChatMessageEngineSchema, WireDieKindSchema, WireRawRollSchema, RecalcHistoryEntrySchema, parseMessageEngine, isKnownSegment, baseRollDice, numericBounds, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
export type { MessageKind, DieRecord, RollOutcome, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm, WireDieKind, WireRawRoll, RecalcHistoryEntry } from "./chat-docs";
```

- [ ] **Step 12: Add `WorldSession.recalcRoll`**

In `src/client/shell/src/lib/worldSession.svelte.ts`, add the import for `WireRecalcOp` alongside this file's existing wire-type imports (locate the existing `import type { ChatSendOptions, ... } from "@shadowcat/core";`-style import and add `WireRecalcOp` to it), then add the method directly after `deleteChatMessage`'s closing `}`:

```ts
  /** GM-only roll correction. Resolves/rejects with the correlated outcome.
   * @param messageId The message carrying the targeted roll.
   * @param rollId The targeted roll's stable id.
   * @param ops The targeted mutation(s) to apply.
   * @returns Same silence-based resolution as `sendChatMessage` -- resolves when the error window
   * elapses with no correlated `chat_error`, rejects on one or on disconnect.
   * @example
   * ```
   * declare const session: WorldSession;
   * await session.recalcRoll("msg-1", "roll-1", [{ kind: "remove_dice", ids: [2] }]);
   * ```
   */
  recalcRoll(messageId: string, rollId: string, ops: WireRecalcOp[]): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.recalcRoll(messageId, rollId, ops);
  }
```

- [ ] **Step 13: Wire `ChatApi.recalc` in `Table.svelte`**

In `src/client/shell/src/lib/Table.svelte`, the existing `chat: {...}` block:

```ts
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
    },
```

becomes:

```ts
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
      recalc: (id, rollId, ops) => session.recalcRoll(id, rollId, ops),
    },
```

- [ ] **Step 14: Extend `ChatApi` in `appContext.ts`**

In `src/client/ui-kit/src/appContext.ts`, extend the import to bring in `WireRecalcOp`:

```ts
import type { TokenSelection } from "./tokenSelection.svelte";
import type { PanelsApi, PanelsChipsView } from "./panelsBridge.svelte";
import type { SceneSelection } from "./sceneSelection.svelte";
```

(add `WireRecalcOp` to the existing large `@shadowcat/core` import on line 2, which already lists `ChatSendOptions`).

Extend the `ChatApi` interface (currently ending with `delete(messageId: string): Promise<void>;`):

```ts
export interface ChatApi {
  send(opts: ChatSendOptions): Promise<void>;
  edit(messageId: string, content: string): Promise<void>;
  delete(messageId: string): Promise<void>;
  /** GM-only roll correction: locate a roll by id and apply targeted die
   * mutations, appending an auditable `recalc_history` entry. Same
   * correlated-rejection contract as `send`/`edit`/`delete`.
   * @param messageId - Id of the message carrying the targeted roll.
   * @param rollId - The targeted roll's stable id.
   * @param ops - The targeted mutation(s) to apply.
   * @returns Resolves once the recalc is accepted; rejects on a non-GM sender
   * or server refusal. */
  recalc(messageId: string, rollId: string, ops: WireRecalcOp[]): Promise<void>;
}
```

- [ ] **Step 15: Update the two test fixture defaults**

In `src/client/ui-kit/src/__fixtures__/appContextTest.ts`, change:

```ts
    chat: over.chat ?? { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve() },
```

to:

```ts
    chat: over.chat ?? {
      send: () => Promise.resolve(),
      edit: () => Promise.resolve(),
      delete: () => Promise.resolve(),
      recalc: () => Promise.resolve(),
    },
```

In `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`, change the inline `chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve() }` to `chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve() }`.

- [ ] **Step 16: Fix the 4 existing per-test `chat: {...}` literals in `MessageCard.test.ts`**

In `src/modules/chat-card/src/MessageCard.test.ts`, each of the 4 call sites currently reading `chat: { send: () => Promise.resolve(), edit, delete: () => Promise.resolve() }` (two occurrences) and `chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: del }` (two occurrences) needs a trailing `recalc: () => Promise.resolve()`:

```ts
chat: { send: () => Promise.resolve(), edit, delete: () => Promise.resolve(), recalc: () => Promise.resolve() }
```

```ts
chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: del, recalc: () => Promise.resolve() }
```

Apply this to all 4 occurrences (the `edit`-focused test at "Save dispatches...", the `edit`-focused test at "Cancel reverts...", and the two `delete`-focused tests at "Delete calls..."/"Delete does not call...").

- [ ] **Step 17: Run the full client + module test/type suites touched so far**

Run: `pnpm --filter @shadowcat/core test`
Run: `pnpm --filter @shadowcat/core test:types` (if this script exists; otherwise `pnpm --filter @shadowcat/core exec tsc --noEmit`)
Run: `pnpm --filter @shadowcat/shell test`
Run: `pnpm --filter @shadowcat/ui-kit test`
Run: `pnpm --filter @shadowcat/module-chat-card test`
Expected: PASS across all five.

- [ ] **Step 18: Commit**

```bash
git add src/client/core/src/chat-docs.ts src/client/core/src/chat-docs.test.ts src/client/core/src/wire.ts src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts src/client/core/src/index.ts src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/Table.svelte src/client/ui-kit/src/appContext.ts src/client/ui-kit/src/__fixtures__/appContextTest.ts src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte src/modules/chat-card/src/MessageCard.test.ts
git commit -m "feat(client): wire mirror + ChatApi.recalc plumbing for RecalcRoll"
```

---

### Task 5: GM recalc UI + non-GM "recalculated" badge

**Files:**
- Modify: `src/modules/chat-card/src/MessageCard.svelte` (block-form die-chip GM menu + non-GM badge)
- Modify: `src/modules/chat-card/src/RollTooltip.svelte` (non-GM badge for inline rolls, passive only -- see decomposition note below)
- Test: `src/modules/chat-card/src/MessageCard.test.ts`
- Test: `src/modules/chat-card/src/RollTooltip.test.ts`

**Interfaces:**
- Consumes: `ctx.chat.recalc(messageId, rollId, ops)` (Task 4), `baseRollDice`/`numericBounds` (Task 4), `ChatSegment`'s `roll_embed.roll_id`/`raw`/`recalc_history` (Task 4).
- Produces: no new exported symbols (UI-only).

**Decomposition note:** the interactive GM recalc menu (reroll/remove/replace-face per die) is scoped to the BLOCK-FORM roll render in `MessageCard.svelte` (the common `/roll` command case: `content.length === 1` and that one segment is `roll_embed`). `RollTooltip.svelte` (the inline-roll popover, for `[[...]]` rolls mixed into Normal/Emote text) gets ONLY the passive "recalculated" indicator, not the interactive menu, to keep this task's scope bounded to one interactive-menu implementation rather than two. Every non-GM recipient of EITHER form still sees the passive indicator when `recalc_history` is non-empty, satisfying spec §5's requirement in full for every recipient; only the GM's INTERACTIVE affordance is block-form-only. Flagged in the plan's final report for visibility.

- [ ] **Step 1: Write a failing test for the GM recalc menu's presence/absence**

Add to `src/modules/chat-card/src/MessageCard.test.ts` (new `describe` block, placed after the existing roll-rendering tests):

```ts
describe("MessageCard — GM recalc menu", () => {
  function rollDoc(overrides: Record<string, unknown> = {}) {
    return msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d6", outcome: rollOutcome(),
        roll_id: "roll-1",
        raw: {
          dice: [{ id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 4 }],
          group_spans: [[0, 1]],
        },
        ...overrides,
      }],
    }));
  }

  it("renders a per-die recalc menu for a GM when raw is present", () => {
    const doc = rollDoc();
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), role: "gm" }),
    });
    expect(container.querySelector(".recalc-menu")).not.toBeNull();
  });

  it("does not render a recalc menu for a non-GM even when raw is present", () => {
    const doc = rollDoc();
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), role: "player" }),
    });
    expect(container.querySelector(".recalc-menu")).toBeNull();
  });

  it("does not render a recalc menu for a GM when raw is absent (non-GM view or legacy roll)", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "1d6", outcome: rollOutcome(), roll_id: "roll-1" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), role: "gm" }),
    });
    expect(container.querySelector(".recalc-menu")).toBeNull();
  });

  it("Reroll calls ctx.chat.recalc with a reroll_dice op for the clicked die", async () => {
    const recalc = vi.fn().mockResolvedValue(undefined);
    const doc = rollDoc();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({
        documents: storeWith(doc),
        role: "gm",
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc },
      }),
    });
    await fireEvent.click(screen.getByText("chat.roll.reroll"));
    expect(recalc).toHaveBeenCalledWith("m1", "roll-1", [{ kind: "reroll_dice", ids: [0] }]);
  });

  it("Remove calls ctx.chat.recalc with a remove_dice op for the clicked die", async () => {
    const recalc = vi.fn().mockResolvedValue(undefined);
    const doc = rollDoc();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({
        documents: storeWith(doc),
        role: "gm",
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc },
      }),
    });
    await fireEvent.click(screen.getByText("chat.roll.remove"));
    expect(recalc).toHaveBeenCalledWith("m1", "roll-1", [{ kind: "remove_dice", ids: [0] }]);
  });

  it("Replace calls ctx.chat.recalc with a replace_die op using the entered natural", async () => {
    const recalc = vi.fn().mockResolvedValue(undefined);
    const doc = rollDoc();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({
        documents: storeWith(doc),
        role: "gm",
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc },
      }),
    });
    const input = screen.getByLabelText("chat.roll.replaceInput") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "6" } });
    await fireEvent.click(screen.getByText("chat.roll.replace"));
    expect(recalc).toHaveBeenCalledWith("m1", "roll-1", [{ kind: "replace_die", id: 0, natural: 6 }]);
  });

  it("renders a recalculated badge when recalc_history is non-empty", () => {
    const doc = rollDoc({
      recalc_history: [{ previous_outcome: rollOutcome({ total: 2 }), recalculated_by: "u-gm", recalculated_at: 100 }],
    });
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), role: "player" }),
    });
    expect(container.querySelector(".chip.recalculated")).not.toBeNull();
  });

  it("does not render a recalculated badge when recalc_history is absent", () => {
    const doc = rollDoc();
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), role: "player" }),
    });
    expect(container.querySelector(".chip.recalculated")).toBeNull();
  });
});
```

Add `chat.roll.reroll`/`chat.roll.remove`/`chat.roll.replace`/`chat.roll.replaceInput` to this test file's existing fake-translation map if the tests above assert on literal translation keys the way other tests in this file do (follow the file's existing `fakeT`/default-`t` convention exactly -- read how `chat.edit`/`chat.delete` resolve in the tests already in this file and mirror it).

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `pnpm --filter @shadowcat/module-chat-card test -- MessageCard`
Expected: FAIL -- `.recalc-menu`/`.chip.recalculated` do not exist yet.

- [ ] **Step 3: Implement the GM recalc menu and recalculated badge in `MessageCard.svelte`**

In `src/modules/chat-card/src/MessageCard.svelte`, add to the imports:

```ts
  import { parseMessageEngine, isKnownSegment, resolveTokenActor, actorDisplayName, baseRollDice, numericBounds, type ChatSegment, type UnknownSegment, type WireActorOwnerRef, type WireDocument } from "@shadowcat/core";
```

Add a per-die replace-input local state map and the recalc dispatch functions, near the existing `editing`/`draft` state declarations:

```ts
  const isGm = $derived(ctx.role === "gm");
  // Per-die "replace with" draft input, keyed by die id -- a plain object
  // (not a Map) so Svelte's reactivity tracks individual key writes.
  let replaceDrafts = $state<Record<number, string>>({});

  /** Sends a single-op recalc for the given die.
   * @param rollId The targeted roll's stable id.
   * @param op The single targeted mutation.
   * @example
   * ```
   * // internal; called from the recalc menu's action buttons
   * sendRecalc("roll-1", { kind: "reroll_dice", ids: [0] });
   * ```
   */
  function sendRecalc(
    rollId: string,
    op: { kind: "reroll_dice"; ids: number[] } | { kind: "replace_die"; id: number; natural: number } | { kind: "remove_dice"; ids: number[] },
  ): void {
    ctx.chat.recalc(message.id, rollId, [op]);
  }
```

Locate the block-form roll rendering (`{#if rollBlock}` ... `<div class="roll-dice"> {#each rollBlock.outcome.records as r, i (i)} ... {/each} </div>`) and, directly after that `</div>` (closing `.roll-dice`) but still inside `.roll-block`, add:

```svelte
            {#if rollBlock.recalc_history?.length}
              <span class="chip recalculated">{t("chat.roll.recalculated")}</span>
            {/if}
            {#if isGm && rollBlock.raw}
              <div class="recalc-menu">
                {#each baseRollDice(rollBlock.raw) as die (die.id)}
                  <div class="recalc-die-row">
                    <span class="recalc-die-natural">{die.natural}</span>
                    <button type="button" onclick={() => sendRecalc(rollBlock!.roll_id!, { kind: "reroll_dice", ids: [die.id] })}>
                      {t("chat.roll.reroll")}
                    </button>
                    <button type="button" onclick={() => sendRecalc(rollBlock!.roll_id!, { kind: "remove_dice", ids: [die.id] })}>
                      {t("chat.roll.remove")}
                    </button>
                    {#if numericBounds(die.kind)}
                      {@const bounds = numericBounds(die.kind)!}
                      <input
                        type="number"
                        aria-label={t("chat.roll.replaceInput")}
                        min={bounds.min}
                        max={bounds.max}
                        value={replaceDrafts[die.id] ?? ""}
                        oninput={(e) => (replaceDrafts[die.id] = (e.currentTarget as HTMLInputElement).value)}
                      />
                      <button
                        type="button"
                        onclick={() => {
                          const n = Number(replaceDrafts[die.id]);
                          if (Number.isFinite(n)) sendRecalc(rollBlock!.roll_id!, { kind: "replace_die", id: die.id, natural: n });
                        }}
                      >
                        {t("chat.roll.replace")}
                      </button>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
```

Add matching styles to the `<style lang="scss">` block:

```scss
  .chip.recalculated {
    font-style: normal;
    opacity: 0.9;
  }
  .recalc-menu {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-top: var(--space-1);
    border-top: 1px dashed var(--border);
  }
  .recalc-die-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .recalc-die-row input[type="number"] {
    width: 4em;
  }
  .recalc-die-row button {
    min-height: 44px;
    padding: 0 var(--space-1);
  }
```

- [ ] **Step 4: Add the three new translation keys to whatever i18n message source this module/consuming app uses**

Locate the same message-source file(s) that already define `chat.edit`/`chat.delete`/`chat.roll.formula` etc. (the project's i18n message catalog -- not owned by this module directly; follow the SAME file(s) those existing keys live in) and add:

```
chat.roll.reroll = "Reroll this die"
chat.roll.remove = "Remove this die"
chat.roll.replace = "Replace this die's face"
chat.roll.replaceInput = "Replacement face value"
chat.roll.recalculated = "Recalculated"
```

- [ ] **Step 5: Run the `MessageCard` test suite**

Run: `pnpm --filter @shadowcat/module-chat-card test -- MessageCard`
Expected: PASS.

- [ ] **Step 6: Write a failing test for `RollTooltip`'s passive badge**

Add to `src/modules/chat-card/src/RollTooltip.test.ts`, matching that file's existing render pattern:

```ts
  it("renders a recalculated badge when recalcHistory is non-empty", () => {
    const { container } = render(RollTooltip, {
      props: {
        outcome: rollOutcome(),
        recalcHistory: [{ previous_outcome: rollOutcome({ total: 2 }), recalculated_by: "u-gm", recalculated_at: 100 }],
      },
    });
    expect(container.querySelector(".chip.recalculated")).not.toBeNull();
  });

  it("renders no badge when recalcHistory is absent", () => {
    const { container } = render(RollTooltip, { props: { outcome: rollOutcome() } });
    expect(container.querySelector(".chip.recalculated")).toBeNull();
  });
```

Match this file's existing `rollOutcome()` helper import/definition exactly (if this test file has its own local helper distinct from `MessageCard.test.ts`'s, use that one; do not import across test files).

- [ ] **Step 7: Run the test to confirm it fails**

Run: `pnpm --filter @shadowcat/module-chat-card test -- RollTooltip`
Expected: FAIL -- `RollTooltip` accepts no `recalcHistory` prop yet.

- [ ] **Step 8: Add the optional `recalcHistory` prop and badge to `RollTooltip.svelte`**

In `src/modules/chat-card/src/RollTooltip.svelte`, change the props block (currently):

```ts
  import type { RollOutcome } from "@shadowcat/core";

  let {
    outcome,
  }: {
    /** The executed roll's full audit record — every die, kept/dropped, plus any labeled constants. */
    outcome: RollOutcome;
  } = $props();
```

to:

```ts
  import type { RollOutcome, RecalcHistoryEntry } from "@shadowcat/core";

  let {
    outcome,
    recalcHistory,
  }: {
    /** The executed roll's full audit record — every die, kept/dropped, plus any labeled constants. */
    outcome: RollOutcome;
    /** Present iff this roll has been recalculated at least once — renders a
     * passive "recalculated" indicator. Never interactive here (see
     * `MessageCard.svelte`'s decomposition note for where the GM recalc menu
     * lives). */
    recalcHistory?: RecalcHistoryEntry[] | null;
  } = $props();
```

Add the badge to the markup, directly after the closing `</button>` of `.roll-tooltip-trigger`:

```svelte
  {#if recalcHistory?.length}
    <span class="chip recalculated">{t("chat.roll.recalculated")}</span>
  {/if}
```

Add matching styles:

```scss
  .chip.recalculated {
    font-size: 0.85em;
    padding: 0 4px;
    border-radius: var(--radius-1);
    border: 1px solid var(--border);
    margin-left: 4px;
  }
```

- [ ] **Step 9: Wire `MessageCard.svelte`'s inline-segment render to pass `recalcHistory` through**

In `MessageCard.svelte`, the inline segment render currently reads:

```svelte
              {:else if s.kind === "roll_embed"}
                <RollTooltip outcome={s.outcome} />
```

Change to:

```svelte
              {:else if s.kind === "roll_embed"}
                <RollTooltip outcome={s.outcome} recalcHistory={s.recalc_history} />
```

- [ ] **Step 10: Run both test suites**

Run: `pnpm --filter @shadowcat/module-chat-card test`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/modules/chat-card/src/MessageCard.svelte src/modules/chat-card/src/RollTooltip.svelte src/modules/chat-card/src/MessageCard.test.ts src/modules/chat-card/src/RollTooltip.test.ts
git commit -m "feat(chat-card): GM recalc menu + recalculated badge for RollEmbed"
```

(Also stage and commit whatever i18n message-source file(s) Step 4 touched, in the same commit.)

---

### Task 6: End-to-end integration test

**Files:**
- Create: `src/client/core/src/e2e/recalc-roll.e2e.test.ts`

**Interfaces:**
- Consumes: `startTestServer`/`login`/`TestServer`/`Fixture` (`src/client/core/src/e2e/server-process.ts`, existing), `WsClient.sendChatMessage`/`recalcRoll` (Task 4), `nodeConnect` pattern (existing, per-file helper -- copy it, matching `name-privacy.e2e.test.ts`'s own local `nodeConnect` function).
- Produces: no exported symbols (a standalone test file).

- [ ] **Step 1: Write the end-to-end test**

Create `src/client/core/src/e2e/recalc-roll.e2e.test.ts`:

```ts
// Node<->Rust end-to-end: a GM's `/roll` produces a RollEmbed with spec/raw
// hidden from a non-GM player but visible to the GM; a GM recalc mutates the
// stored outcome, appends a visible-to-everyone recalc_history entry, and the
// player's redacted view still never receives spec/raw (including inside the
// new history entry's previous_raw).
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { WireCommand } from "../wire";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => {
  server = await startTestServer();
});
afterAll(() => server?.stop());

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () =>
        resolve({ send: (d: string) => sock.send(d), close: () => sock.close() }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("GM recalc: spec/raw stay GM-only, recalc_history is visible to everyone and never leaks previous_raw to a player", async () => {
  const gmCookie = await login(server.baseUrl, server.fixture.gm, "pw");
  const plCookie = await login(server.baseUrl, server.fixture.player, "pw");
  const world = server.fixture.world;

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  let messageId = "";
  let rollId = "";
  let sawGmRaw = false;
  const gmWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const content = (op.doc.engine as { content: { kind: string; roll_id?: string; raw?: unknown }[] }).content;
            const embed = content.find((s) => s.kind === "roll_embed");
            if (embed?.roll_id) {
              messageId = op.doc.id;
              rollId = embed.roll_id;
              if (embed.raw) sawGmRaw = true;
            }
          }
        }
      },
    },
  });
  await gmWatch.start();
  await sleep(300);

  let playerSawRaw: unknown = "unset";
  const playerWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const content = (op.doc.engine as { content: { kind: string; raw?: unknown }[] }).content;
            const embed = content.find((s) => s.kind === "roll_embed");
            if (embed) playerSawRaw = embed.raw ?? null;
          }
        }
      },
    },
  });
  await playerWatch.start();
  await sleep(300);

  await gm.sendChatMessage({ channel: "all", content: "/roll 1d6" });
  await sleep(500);

  expect(messageId).not.toBe("");
  expect(rollId).not.toBe("");
  expect(sawGmRaw).toBe(true);
  expect(playerSawRaw).toBeNull();

  // Recalc as GM.
  let playerSawRecalcHistory: unknown[] | null = null;
  let playerSawRecalcHistoryRaw: unknown = "unset";
  const playerWatch2 = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "update" && op.doc_id === messageId) {
            const engineChange = op.changes.find((c) => c.path === "/engine");
            if (engineChange && typeof engineChange.new === "object" && engineChange.new !== null) {
              const content = (engineChange.new as { content: { kind: string; recalc_history?: { previous_raw?: unknown }[] }[] }).content;
              const embed = content.find((s) => s.kind === "roll_embed");
              if (embed?.recalc_history) {
                playerSawRecalcHistory = embed.recalc_history;
                playerSawRecalcHistoryRaw = embed.recalc_history[0]?.previous_raw ?? null;
              }
            }
          }
        }
      },
    },
  });
  await playerWatch2.start();
  await sleep(300);

  await gm.recalcRoll(messageId, rollId, []);
  await sleep(500);

  expect(playerSawRecalcHistory).not.toBeNull();
  expect((playerSawRecalcHistory as unknown[]).length).toBe(1);
  expect(playerSawRecalcHistoryRaw).toBeNull();

  gm.stop();
  gmWatch.stop();
  playerWatch.stop();
  playerWatch2.stop();
});
```

Note: read the exact current shape of `WireCommand`/`Operation`'s `update` variant (field names `doc_id`/`changes`/`path`/`new`) in `src/client/core/src/wire.ts` before finalizing this test -- this plan's research confirmed `WireFieldChange` carries `old`/`new`/`path`/`remove` and `Operation`'s `update` variant carries `doc_id`/`changes`, but the implementer should re-verify field names compile against the current `wire.ts` types exactly.

- [ ] **Step 2: Run the e2e test**

Run: `pnpm --filter @shadowcat/core test -- e2e/recalc-roll`
Expected: PASS. (This test spins up a real server process; if it hangs, check that `server.fixture.gm`/`server.fixture.player` usernames and the `"pw"` password match the seeded e2e fixture's actual credentials by reading `server-process.ts`'s fixture-seeding code before debugging further.)

- [ ] **Step 3: Run the full e2e suite to confirm no regression**

Run: `pnpm --filter @shadowcat/core test -- e2e`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/client/core/src/e2e/recalc-roll.e2e.test.ts
git commit -m "test(e2e): GM recalc-from-chat flow + spec/raw redaction"
```

---

### Task 7: Codebase skill doc-sync pass

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-dice/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md` (only if Task 3's `apply_intent` exact-path admission needs a mention there — see Step 3)

**Interfaces:** none (documentation only).

- [ ] **Step 1: Update `shadowcat-codebase-chat`**

In `.claude/skills/shadowcat-codebase-chat/SKILL.md`:

- Under "Dice wire — `chat::rolls` + the ingest roll stage", update the bullet describing `handle_send_message`'s roll stage to note that `Segment::RollEmbed` now also carries `roll_id`/`spec`/`raw`/`recalc_history`, and that `spec`/`raw` are populated (not discarded) so a GM can later recalculate via the new `handle_recalc_roll`.
- Add a new bullet (or extend the existing "Roll immutability" bullet's neighborhood) documenting: `handle_recalc_roll` is GM-only (never owner-or-GM, unlike edit/delete), reuses `WriteOrigin::ServerMessageRevision` as its third caller, and locates a roll by `roll_id` (stable identity, never array index).
- Update the "Key files & seams" `apply_intent` bullet (the "FOUR coupled chokepoints" list) to describe the new exact-path admission for `/permissions/property_overrides` under `MESSAGE_DOC_TYPE` + `ServerMessageRevision`, and why it does NOT grant `cap::EDIT_PERMISSIONS` broadly (the same reasoning captured in this plan's "Spec-vs-codebase note").
- Update the module-level doc-comment quote/description (if the skill quotes it) to match the updated `chat/mod.rs` module doc from Task 3, Step 5.
- Add a new Hard Invariants bullet: "`spec`/`raw` on a `RollEmbed` (and every `RecalcEntry.previous_raw`) are GM-only via `permissions.property_overrides`, populated by `roll_embed_property_overrides` at message-Create time and re-populated on every recalc — never a chat-specific redaction filter; `outcome`/`recalc_history`/`roll_id` stay visible to every recipient."
- Update the "Client display layer" section's `MessageCard`/`RollTooltip` description to mention the new GM recalc menu (block-form only) and the passive "recalculated" badge (both forms).

- [ ] **Step 2: Update `shadowcat-codebase-dice`**

In `.claude/skills/shadowcat-codebase-dice/SKILL.md`:

- Under "Hard invariants", update the bullet "Pure library — `dice` must never depend on `ws`/`data`/`http`/`scene`. Still NO wire frames and NO `#[derive(TS)]`/ts-rs bindings: roll outcomes ride the opaque chat `system` body..." to note that `RecalcOp` now derives `Serialize`/`Deserialize` (for storage inside `chat::RecalcEntry`) but still carries NO `TS` derive — the wire-facing mirror (`chat::WireRecalcOp`) lives in `chat`, not `dice`, preserving the crate boundary.
- Under "Key files & seams" → `dice::recalc`, add a short note that `chat::rolls::execute_roll`/`execute_roll_with_seed` now return the `RollSpec`/`RawRoll` alongside `formula`/`outcome`, and that `chat::handle_recalc_roll` (in `chat`, not `dice`) is `recalculate`'s first production caller.

- [ ] **Step 3: Decide whether `shadowcat-codebase-documents-permissions` needs an update**

Re-read that skill's "Hard invariants" `apply_intent`-related bullets (the `ServerMessageRevision`-adjacent ones live under `shadowcat-codebase-chat`, not this skill, per that skill's own "First (and so far only) consumer" note for `gm_role`). If, on re-reading, the exact-path `/permissions/property_overrides` admission is judged to belong in THIS skill instead (since it touches `data::sqlite::apply_intent`, owned by this skill's subsystem) rather than `shadowcat-codebase-chat`, add a short Hard Invariants bullet there instead of (or in addition to) the `shadowcat-codebase-chat` one from Step 1, cross-referencing the other skill. State explicitly in the review dispatch (Step 4) which skill ended up owning this fact, so the reviewer checks the right one.

- [ ] **Step 4: Dispatch `shadowcat-spec-reviewer` on the skill diff**

Per this project's CLAUDE.md "Reviewed Skill-Update Gate," dispatch `shadowcat-spec-reviewer` (`effort: high`) with the diff of all skill files touched in Steps 1-3 plus the actual code diff from Tasks 1-6, asking it to confirm: (a) no omission — every new seam/invariant/gotcha from Tasks 1-6 is reflected; (b) no drift — every sentence matches the merged code exactly; (c) no broken pointer — every cross-reference (e.g. "see `handle_recalc_roll`") resolves to a real symbol.

- [ ] **Step 5: Address any findings, then commit**

```bash
git add .claude/skills/shadowcat-codebase-chat/SKILL.md .claude/skills/shadowcat-codebase-dice/SKILL.md .claude/skills/shadowcat-codebase-documents-permissions/SKILL.md
git commit -m "docs(skills): sync chat/dice/documents-permissions skills for recalc-from-chat"
```

- [ ] **Step 6: Push**

This is the final task of the milestone. Once Task 7 is committed and CI is green on all three OSes:

```bash
git push origin <branch>
gh run watch
```
