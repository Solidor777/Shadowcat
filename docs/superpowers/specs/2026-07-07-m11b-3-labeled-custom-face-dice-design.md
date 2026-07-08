# M11b-3 — Labeled + Custom-Face Dice — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Scope:** M11b-3 only — the die-model rules (§9 of the M11b system-rules design): labeled dice and
custom-face (symbolic) dice, plus their notation and outcome surface.
**Builds on:** M11a + M11b-1 + M11b-2 (all merged, stable, local `main`). Parent design:
[`2026-07-04-m11b-system-rules-design.md`](2026-07-04-m11b-system-rules-design.md) §9–§13.
**Consumer:** M11d wires dice to transport/chat and adds ts-rs bindings; **not** in scope here.

This document resolves the detail-level decisions §9 left underspecified. It is the final M11b
checkpoint; after it the dice engine is feature-complete as a pure library. Two decisions here are
*tighter* than §9's literal text and are called out as such (§F expertise scope, §A label charset)
per the project's strict spec-adherence discipline — both consented at brainstorm.

## 1. Build decomposition

**One plan / one branch / one merge / one `shadowcat-codebase-dice` gate.** Tasks sequence
**labels first** (trivial — a propagated string field + two read helpers), then custom-face dice
(medium — a new `DieKind` variant touching RNG, the success predicate, the crit path, `fold`, and
several guard sites). The two features share `RollOutcome`/`DieRecord` surface, so a single coherent
diff is cleaner than splitting; labels being trivial won't bloat the plan.

Pure library — `dice` must never depend on `ws`/`data`/`http`/`scene`; no wire frames, no ts-rs
(M11d). All M11a/b-1/b-2 invariants remain binding and unchanged.

## 2. What this extends (stable base)

- `DieKind::Numeric { min, max }` (numeric-only today).
- `SuccessRule { comp: Comparator, target: i32 }` — a *struct*, a required field on `SuccessConfig`.
- `CritSuccess { threshold: i32, extra_successes, positive_counter }` /
  `CritFail { threshold: i32, lost, negative_counter, allow_negative }` — numeric thresholds scored
  by `crit::score_die(direction, value: i32, cfg)` via the direction-aware `reaches()`.
- `DiceGroup { count, kind, modifiers }` — no `label`.
- `DieRecord { id, group_index, natural, value, kept, exploded, rerolled_from, crit_success,
  crit_fail, expertise }`.
- `RollOutcome { total, records, successes, pass, margin, tier_label, tier_value, crit_successes,
  crit_fails, positive_counter, negative_counter }`.

`roll` (only RNG step), `evaluate` (pure), `recalculate` (targeted ops → re-derive) keep their
three-function contract; this changes what `evaluate` reads and emits.

## 3. Labeled dice (Tasks 1–2 — low risk)

- **Data model:** `DiceGroup` gains `label: Option<String>`, propagated onto every `DieRecord` the
  group produces — including explode/penetrate children, at the same construction sites that stamp
  `group_index` (`resolve_group`/`push_extra`). Any mode (orthogonal to Total/SuccessCount).
- **Outcome helpers** (public API — gated by CLAUDE.md §2):
  ```rust
  impl RollOutcome {
      fn by_label(&self, label: &str) -> Vec<&DieRecord>;
      fn compare_labels(&self, a: &str, b: &str) -> Option<Ordering>;
  }
  ```
  - `by_label` returns the kept-and-unkept records carrying that label (callers filter `.kept` if
    needed; exposing all is more general).
  - `compare_labels` compares the two labels by **sum of kept `value`s** (consistent with how Total
    mode aggregates a group), **direction-independent** ("which rolled higher" literally — the
    *system* maps `Greater`/`Less`/`Equal` to meaning). Returns `None` if either label is absent
    **or unordered** (a symbolic group with no defined ordering; see §5). Daggerheart Hope/Fear:
    `Greater` ⇒ Hope, `Less` ⇒ Fear, `Equal` ⇒ both.

### 3.1 Notation for labels
`1d12[Hope] + 1d12[Fear]`. `[...]` attaches to a dice group (immediately after the group's dice +
modifiers). Lowering: sets `DiceGroup.label`.

**Charset (decision — tighter than §9's bare `[label]`):** a label is ASCII printable characters
**except `]`**, with surrounding whitespace trimmed, and must be **non-empty** after trimming
(`ParseError::EmptyLabel`). An unterminated `[` is `ParseError::UnterminatedLabel`. A **duplicate
label across groups is NOT an error** — two groups may intentionally share a label and pool under
`by_label`. The lexer's existing ASCII-only precondition already covers the byte range; `[`/`]`
become new bracket tokens (or a single `Label(String)` token — pinned in the plan). Full face-lists
remain struct-only (parent §10); notation constructs only labels, never `Faces`.

## 4. Custom-face (symbolic) dice — data model (Tasks 3–4)

```rust
enum DieKind {
    Numeric { min: i32, max: i32 },      // unchanged
    Faces   { faces: Vec<Face> },        // NEW
}
struct Face { value: Option<i32>, symbols: Vec<Symbol> }
type Symbol = String;                     // opaque tag; the system assigns meaning
```

- **RNG.** A `Faces` die draws a uniform **index** `0..faces.len()` (one `roll_uniform(0,
  len-1)` draw); `RawDie.natural` stores that **index** (the same field a `Numeric` die uses for its
  face value — reused, not widened). `evaluate` derives, by looking the index up in `kind.faces`:
  - `value = faces[natural].value.unwrap_or(0)` → stored in `DieRecord.value` (stays `i32`; no
    churn). A `None`-value face contributes `0` numerically; its payload is `symbols`.
  - `symbols = faces[natural].symbols.clone()` → stored in `DieRecord.symbols`.
- **Validation (release-safe guard).** A `Faces { faces }` with `faces.is_empty()` is invalid: a
  `Faces` die draws `roll_uniform(0, faces.len() - 1)`, and `roll_uniform` only `debug_assert!`s a
  non-degenerate range (unsafe in release), exactly the `sides >= 1` hazard `Numeric` already
  documents. No notation constructs `Faces` in M11b-3 (struct-only, parent §10), so the guard is a
  `DieKind`/`RollSpec` `validate()` helper callable at any construction boundary — tests use it, and
  it becomes the enforcement point when M11d's untrusted wire path builds a `RollSpec`. The
  release-safe untrusted-input enforcement is deferred to that M11d boundary alongside `sides >= 1`
  and the dice-count cap (§13).

## 5. The `is_ordered` predicate (Task 5) — one gate for value-reading ops

```rust
impl DieKind {
    /// A die participates in value-based operations iff its faces have a defined ordering.
    fn is_ordered(&self) -> bool {
        match self {
            DieKind::Numeric { .. } => true,
            DieKind::Faces { faces } => faces.iter().all(|f| f.value.is_some()),
        }
    }
}
```

**One predicate gates every value-*reading* stage.** Ordered (`Numeric`, or `Faces` with **every**
face valued) behaves exactly like a numeric die for:

- `eval::sum::fold` — its `value` folds into `total`;
- keep/drop-highest/lowest — ranked by `value`;
- comparator `Explode`/`Reroll` — the trigger tests `value`.

Unordered (`Faces` with **any** `value: None`) → contributes **0** to `total` and is **fail-closed
skipped** by keep/drop-high/low and comparator explode/reroll; its only payload is `symbols`. A
mixed valued/None face-list is unordered (a `None` face can't be ranked against a valued one) — the
`all(..)` rule captures this. The guard is checked at each site against the group's `kind`, never
inferred from `DieRecord.value` (a genuine face value of `0` must not read as "unordered").

## 6. Success predicate + crits (Tasks 6–7 — crit task buddy-checked)

```rust
enum SuccessRule {                                  // was a struct
    #[default]
    Numeric { comp: Comparator, target: i32 },      // default arm — preserves M11b-1 behavior
    HasSymbol(Symbol),
}

enum CritTrigger { AtLeast(i32), HasSymbol(Symbol) }

struct CritSuccess { trigger: CritTrigger, extra_successes: i32, positive_counter: i32 }
struct CritFail    { trigger: CritTrigger, lost: i32, negative_counter: i32, allow_negative: bool }
```

- **`SuccessRule` defaults to `Numeric`.** `Comparator` gains `#[derive(Default)]` +
  `#[default] Gte` so the `Numeric` default arm is well-formed (`target` defaults to `0`). Any
  `Default`- or serde-defaulted `SuccessConfig` therefore gets a numeric rule, never a symbol rule —
  principle of least surprise, and no silent behavior change for existing numeric configs. Existing
  numeric constructions churn mechanically to `SuccessRule::Numeric { .. }`.
- **Per-die success test:** `match rule { Numeric { comp, target } => comp.test(value, target),
  HasSymbol(s) => record.symbols.contains(s) }`. A symbolic pool thus feeds base successes → net →
  margin → the shared tier/pass classifier — the whole classification stack becomes available to
  symbolic dice, not just raw tallies.
- **Symbol crits (full §9 fidelity).** `crit::score_die` takes the die's `symbols` (signature moves
  from `value: i32` to `&DieRecord`, or `(value, &[Symbol])` — pinned in the plan). `reaches()` for
  `HasSymbol` is `symbols.contains(s)` and is **direction-insensitive** (a symbol is present or
  absent — there is no "better end" to flip); `AtLeast(i32)` remains direction-aware exactly as
  today. Genesys Triumph = `CritSuccess { trigger: HasSymbol("triumph"), extra_successes: 1,
  positive_counter: 1 }`; Despair symmetric on `CritFail`. Crit deltas fold into net successes and
  the positive/negative counters unchanged (parent §7). **This reopens the sealed, buddy-check-tier
  crit path — the crit task carries a pre-authorized buddy-check.**

## 7. Tallies & outcome additions (folded into the tasks above)

```rust
struct RollOutcome {   // M11b-1/b-2 fields, plus:
    symbol_counts: BTreeMap<Symbol, i32>,   // over KEPT dice; computed unconditionally, deterministic
    // by_label / compare_labels are methods (§3), not fields
}
struct DieRecord {     // M11a/b-2 fields, plus:
    label:   Option<String>,   // §3 — propagated from the group
    symbols: Vec<Symbol>,      // §4 — resolved for Faces dice; empty for Numeric
}
```

`symbol_counts` sums each kept die's `symbols` into a `BTreeMap` (ordering deterministic for
reproducibility). It is computed for **every** roll, independent of the `SuccessRule` variant, so a
system can always net symbols out-of-band ("engine exposes, system decides"). The `symbol_counts`
counter role subsumes Genesys advantage/threat tallying; symbol crits (§6) additionally fold into
net successes for systems that want a symbol to drive classification.

## 8. Expertise stays Numeric-only (§F — decision, tighter than §9)

§9's literal guard excludes only `value: None` faces from expertise. But `eval::expertise::adjust`
moves a face `+k` steps *within a contiguous `[min, max]`* toward "better" — a semantics defined
**only** for `DieKind::Numeric`. An arbitrary ordered face-list (`[0,0,1,2,2,3]`) has no contiguous
numeric range; "+1 toward better" is ambiguous (increment the value vs. advance to the next face
index), and mutating `value` to a non-face integer would desync the die's `symbols`. Therefore:

- **Expertise is guarded to `DieKind::Numeric`.** `eval::expertise::allocate`'s
  contributing-die filter selects `Numeric` dice only; ordered `Faces` dice participate in the
  read-based ops (fold, keep/drop, explode, reroll) but are **excluded from expertise**.

This is a *tighter* guard than §9 and a correctness call: it keeps the provably-optimal, oracle-
verified DP operating only where its `adjust`/`v_i(k)` model is sound. The M11b-2 differential
oracle and its invariants are otherwise untouched (the allocator's DP recurrence and tie-break do
not change — only which dice enter the contributing set).

## 9. Recalculate (unaffected structurally)

`recalculate` reconstructs each group's base naturals from `group_spans`, applies ops, then
re-derives. For a `Faces` die a base natural is a **face index**: `RecalcOp::ReplaceDie` replaces an
index (validated `< faces.len()` on the targeted group's kind), `RerollDice` re-draws a fresh index,
`RemoveDice` is unchanged. The M11a invariant that recalc targets only base naturals (never
explode/penetrate children) holds identically.

## 10. Evaluation pipeline (updated)

Per group (`resolve_group`, unchanged M11a mechanics; `is_ordered` gates value-reading modifiers):
reroll → explode → keep/drop. Then per mode:

- **Total:** `eval::sum::fold` sums each **ordered** kept die's `value` (unordered `Faces` add 0) →
  `total` → classify `(total − difficulty, tiers)`.
- **SuccessCount:** pool kept dice across groups → **expertise DP over `Numeric` dice only** (§8,
  adjusts values) → count base successes via the per-die `SuccessRule` (numeric or `HasSymbol`) +
  crit deltas via `CritTrigger` (numeric or `HasSymbol`) + counters + **`symbol_counts` over all
  kept dice** → net successes → classify `(net_successes − required_successes, tiers)`.

All numeric comparisons read `direction`; symbol predicates are direction-insensitive.

## 11. Testing strategy

Pure library, exhaustive unit + property tests (no wire frames):

- **Labels:** `by_label` collects a group's records (incl. explode children); `compare_labels`
  sum-based ordering, `None` on missing/unordered; label notation round-trips; duplicate labels
  pool; `EmptyLabel`/`UnterminatedLabel` parse errors.
- **Custom faces:** face-index RNG uniformity (histogram over a seeded corpus); `value`/`symbols`
  derivation from index; `None`-value face contributes 0 to total and carries symbols; the
  `validate()` helper rejects an empty face-list.
- **`is_ordered` guards:** an unordered `Faces` die is skipped by keep/drop-high/low, comparator
  explode/reroll, and fold (adds 0); an all-valued `Faces` die participates in each exactly like
  `Numeric`.
- **Success + crits over symbols:** `HasSymbol` success feeds net → tier; Triumph symbol-crit adds
  success + positive_counter; Despair symmetric; `SuccessRule`/`CritTrigger` default to numeric.
- **`symbol_counts`:** unconditional per-symbol tallies over kept dice; deterministic ordering.
- **Expertise exclusion:** an ordered `Faces` die never receives expertise points even when spending
  would raise net successes (guarded to `Numeric`); M11b-2 oracle unchanged for `Numeric`-only
  corpora.

Existing M11a/b property tests (direction-flip mirror symmetry, recalc empty-ops identity,
replace-then-replace-back round-trip) extend over the new `label`/`symbols` fields.

## 12. Codebase-skill gate

Update `shadowcat-codebase-dice` (new `DieKind::Faces`, `SuccessRule`/`CritTrigger` enums, the
`is_ordered` gate, symbol success/crit + `symbol_counts`, labeled dice + notation, expertise
Numeric-only guard) and dispatch `shadowcat-spec-reviewer` on the skill diff per the reviewed
skill-update gate — same tier as the doc-sync gate, before merge. The crit-path task additionally
carries a pre-authorized buddy-check (§6).

## 13. Deferred (unchanged from parent §14)

- ts-rs bindings + wire frames → M11d.
- Per-roll dice/face-count caps + `Faces`-length DoS bounding → M11d untrusted-transport boundary
  (alongside the existing dice-count/expertise caps in `docs/TODO.md`).
- Symbol-based **explode/reroll** triggers (a symbol driving a group modifier) — not in §9's scope;
  comparator modifiers stay numeric here. Logged as a candidate follow-up if a real consumer needs
  it (e.g. Genesys advantage-triggered re-rolls).
- Arbitrary-code modder roll handlers; animated/GIF dice; any client-side eval engine.
