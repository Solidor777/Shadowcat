# M11b-3 — Labeled + Custom-Face Dice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship labeled dice and custom-face (symbolic) dice — the final M11b checkpoint — as specified in `docs/superpowers/specs/2026-07-07-m11b-3-labeled-custom-face-dice-design.md`.

**Architecture:** Extend the sealed M11a/b-1/b-2 dice engine with (a) an optional `label` tag on `DiceGroup`/`DieRecord` plus two `RollOutcome` read helpers, and (b) a new `DieKind::Faces` variant with a face-index RNG draw, an `is_ordered` gate protecting every value-reading pipeline stage, a `SuccessRule`/`CritTrigger` enum split (numeric vs symbol), and an unconditional `symbol_counts` tally. Pure library — no wire frames, no ts-rs.

**Tech Stack:** Rust (server crate), no new dependencies. `cargo test -p shadowcat-server` (or the workspace's actual crate name — confirm via `Cargo.toml` before Task 1) is the verification command for every task.

## Global Constraints

- Pure library: `src/server/src/dice/**` must not depend on `ws`/`data`/`http`/`scene`.
- No `#[derive(TS)]`/ts-rs bindings — deferred to M11d.
- `roll` stays the only randomness step; `evaluate` stays pure given `(spec, raws)`.
- Every new/changed public type keeps `Serialize, Deserialize` parity with its siblings (`#[serde(default)]` on additive fields so old JSON still deserializes).
- Field literal order never matters in Rust struct literals — mechanical fixes below may add a field anywhere in the literal.
- The crit-path task (Task 8) is buddy-check-gated per the design's §6/§12 — do not merge until a buddy-check has run on that task's diff.
- Run `cargo build --tests` (workspace) after each task's structural change, before writing new test assertions, to confirm the compile-error set matches what the task expects — do not leave stray unrelated compile errors for a later task to discover.

---

## Task 1: Labeled dice — data model & propagation

**Files:**
- Modify: `src/server/src/dice/spec.rs:64-69` (`DiceGroup`)
- Modify: `src/server/src/dice/outcome.rs:49-67` (`DieRecord`)
- Modify: `src/server/src/dice/eval/groups.rs:22-44` (`resolve_group`'s initial per-natural map), `:163-187` (`push_extra`), and its two call sites at `:102-109` (Penetrate) and `:112-119` (Standard)
- Modify (mechanical compile-fix — add `label: None,` to each): every other `DiceGroup { .. }` literal and every other `DieRecord { .. }` literal in the crate (exact site list in Step 5)
- Test: `src/server/src/dice/eval/groups.rs` (new test in the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `DiceGroup.label: Option<String>`, `DieRecord.label: Option<String>` — consumed by Task 2's `by_label`/`compare_labels` and Task 3's notation.
- Consumes: nothing new (existing `resolve_group(group, group_index, naturals, rng, raws) -> Vec<DieRecord>` signature unchanged).

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/eval/groups.rs`'s `#[cfg(test)] mod tests` (near `group_index_propagates_to_every_record_including_exploded_children`):

```rust
#[test]
fn label_propagates_to_every_record_including_exploded_children() {
    // Same fixture shape as the group_index propagation test: a single die at
    // max that explodes once, so the label must reach BOTH the original
    // record and the pushed exploded child.
    let naturals = vec![d6(0, 6)];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 1,
        group_spans: vec![],
    };
    let g = DiceGroup {
        count: 1,
        kind: DieKind::Numeric { min: 1, max: 6 },
        modifiers: vec![GroupModifier::Explode {
            kind: ExplodeKind::Standard,
            comp: Comparator::Gte,
            target: 6,
        }],
        label: Some("Hope".to_string()),
    };
    let mut rng = ScriptedRng::new(vec![face_x(6), face_x(3)]);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    assert_eq!(recs.len(), 3, "1 original + 2 chained extras");
    assert!(
        recs.iter().all(|r| r.label.as_deref() == Some("Hope")),
        "label must propagate to the original AND every exploded child record"
    );
}
```

- [ ] **Step 2: Run test to verify it fails to compile**

Run: `cargo test -p shadowcat-server label_propagates_to_every_record --lib`
Expected: compile error — `no field 'label' on type 'DiceGroup'` (and later, once that's fixed, `on type 'DieRecord'`). This is the expected red state for a data-model addition (there is no runtime behavior to fail against yet).

- [ ] **Step 3: Add the `label` field to `DiceGroup` and `DieRecord`**

In `src/server/src/dice/spec.rs`, change:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceGroup {
    pub count: u32,
    pub kind: DieKind,
    /// Applied in vec order: reroll/explode alter the die set, keep/drop select from it.
    pub modifiers: Vec<GroupModifier>,
}
```

to:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceGroup {
    pub count: u32,
    pub kind: DieKind,
    /// Applied in vec order: reroll/explode alter the die set, keep/drop select from it.
    pub modifiers: Vec<GroupModifier>,
    /// Optional tag propagated onto every `DieRecord` this group produces
    /// (including exploded/penetrated children). Orthogonal to mode.
    /// `RollOutcome::by_label`/`compare_labels` read this. `None` = unlabeled.
    #[serde(default)]
    pub label: Option<String>,
}
```

In `src/server/src/dice/outcome.rs`, add to `DieRecord` (after `pub expertise: i32,`):

```rust
    /// Tag copied from the producing `DiceGroup.label` (M11b-3); `None` if the
    /// group is unlabeled. Read by `RollOutcome::by_label`/`compare_labels`.
    #[serde(default)]
    pub label: Option<String>,
```

- [ ] **Step 4: Thread `label` through `resolve_group` and `push_extra`**

In `src/server/src/dice/eval/groups.rs`, change the initial per-natural map (inside `resolve_group`):

```rust
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| DieRecord {
            id: d.id,
            group_index,
            natural: d.natural,
            value: d.natural,
            kept: true,
            exploded: false,
            rerolled_from: None,
            crit_success: false,
            crit_fail: false,
            expertise: 0,
            label: group.label.clone(),
        })
        .collect();
```

Change `push_extra`'s signature and body:

```rust
fn push_extra(
    recs: &mut Vec<DieRecord>,
    raws: &mut RawRoll,
    kind: DieKind,
    group_index: usize,
    label: Option<String>,
    natural: i32,
    value: i32,
) {
    let id = raws.push(kind, natural);
    recs.push(DieRecord {
        id,
        group_index,
        natural,
        value,
        kept: true,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label,
    });
}
```

Update both call sites inside the `Explode` match arm:

```rust
                                ExplodeKind::Compound => {
                                    recs[i].value += extra;
                                }
                                ExplodeKind::Penetrate => {
                                    let value = extra - 1;
                                    push_extra(
                                        &mut recs,
                                        raws,
                                        group.kind.clone(),
                                        group_index,
                                        group.label.clone(),
                                        extra,
                                        value,
                                    );
                                }
                                ExplodeKind::Standard => {
                                    push_extra(
                                        &mut recs,
                                        raws,
                                        group.kind.clone(),
                                        group_index,
                                        group.label.clone(),
                                        extra,
                                        extra,
                                    );
                                }
```

- [ ] **Step 5: Fix every other compile site (mechanical)**

Adding a required field with no `Default` impl on `DiceGroup`/`DieRecord` breaks every existing struct literal in the crate. Fix each by adding `label: None,` anywhere inside the literal (field order is irrelevant in Rust). The exact current sites:

`DiceGroup { .. }` literals needing `label: None,` (32 sites, excluding the struct definition and the Task-1-modified `groups.rs` call sites which get `group.label.clone()` instead — those are Step 4, not this step):
- `src/server/src/dice/proptests.rs:13,40,142`
- `src/server/src/dice/recalc.rs:176,250,341`
- `src/server/src/dice/spec.rs:200,220`
- `src/server/src/dice/notation/parser.rs:170,301`
- `src/server/src/dice/eval/groups.rs:205,275,299,341,379,415,447,480,517,549` (test-module literals — NOT the two production sites already fixed in Step 4)
- `src/server/src/dice/eval/mod.rs:65`
- `src/server/src/dice/eval/success.rs:92,139,160,286,330,377,398,432,490,515`
- `src/server/src/dice/eval/sum.rs:84`

`DieRecord { .. }` literals needing `label: None,` (2 sites — the two production sites in `groups.rs` were fixed in Step 4):
- `src/server/src/dice/eval/expertise.rs` (inside the `raws_of` test helper)
- `src/server/src/dice/eval/success.rs` (inside the `manual_raws` test helper)

Run `cargo build --tests -p shadowcat-server` and fix each reported "missing field `label`" error at the reported location until the build is clean. Do not fix any *other* class of error in this step — only `label`-related ones (later tasks will surface their own).

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p shadowcat-server label_propagates_to_every_record --lib`
Expected: PASS.

- [ ] **Step 7: Run the full dice test suite to confirm no regressions**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS (every pre-existing test still holds — `label` defaults to `None` everywhere it isn't explicitly set).

- [ ] **Step 8: Commit**

```bash
git add src/server/src/dice/spec.rs src/server/src/dice/outcome.rs src/server/src/dice/eval/groups.rs src/server/src/dice/eval/mod.rs src/server/src/dice/eval/sum.rs src/server/src/dice/eval/success.rs src/server/src/dice/eval/expertise.rs src/server/src/dice/recalc.rs src/server/src/dice/notation/parser.rs src/server/src/dice/proptests.rs
git commit -m "feat(dice/m11b-3): add label to DiceGroup/DieRecord, propagate through resolve_group"
```

---

## Task 2: `RollOutcome::by_label` + `compare_labels`

**Files:**
- Modify: `src/server/src/dice/outcome.rs` (add an `impl RollOutcome` block)
- Test: `src/server/src/dice/outcome.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `DieRecord.label` (Task 1), `RollOutcome.records: Vec<DieRecord>` (existing).
- Produces: `RollOutcome::by_label(&self, label: &str) -> Vec<&DieRecord>`, `RollOutcome::compare_labels(&self, a: &str, b: &str) -> Option<std::cmp::Ordering>` — consumed by no other task in this plan, but is the permanent public API surface for M11d/system code.

- [ ] **Step 1: Write the failing tests**

Add to `src/server/src/dice/outcome.rs`'s `#[cfg(test)] mod tests`:

```rust
fn labeled_record(label: &str, value: i32, kept: bool) -> DieRecord {
    DieRecord {
        id: 0,
        group_index: 0,
        natural: value,
        value,
        kept,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label: Some(label.to_string()),
    }
}

#[test]
fn by_label_collects_only_matching_records() {
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record("Hope", 5, true),
            labeled_record("Fear", 3, true),
            labeled_record("Hope", 2, true),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
    };
    let hope: Vec<i32> = out.by_label("Hope").iter().map(|r| r.value).collect();
    assert_eq!(hope, vec![5, 2]);
    assert!(out.by_label("Nope").is_empty());
}

#[test]
fn compare_labels_orders_by_sum_of_kept_values() {
    use std::cmp::Ordering;
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record("Hope", 5, true),
            labeled_record("Hope", 1, false), // dropped: excluded from the sum
            labeled_record("Fear", 3, true),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
    };
    // Hope kept-sum = 5, Fear kept-sum = 3 -> Hope > Fear.
    assert_eq!(out.compare_labels("Hope", "Fear"), Some(Ordering::Greater));
    assert_eq!(out.compare_labels("Fear", "Hope"), Some(Ordering::Less));
    assert_eq!(out.compare_labels("Hope", "Missing"), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server by_label_collects_only_matching_records compare_labels_orders_by_sum --lib`
Expected: FAIL with "no method named `by_label`/`compare_labels` found".

- [ ] **Step 3: Implement the helpers**

Add to `src/server/src/dice/outcome.rs` (below the `RollOutcome` struct definition, before `RollResult`):

```rust
impl RollOutcome {
    /// All records (kept and dropped) carrying `label`, in roll order.
    pub fn by_label(&self, label: &str) -> Vec<&DieRecord> {
        self.records
            .iter()
            .filter(|r| r.label.as_deref() == Some(label))
            .collect()
    }

    /// Compares two labels by the sum of their KEPT records' `value`s.
    /// `None` if either label has no records, or either label's records are
    /// unordered (a symbolic group with no numeric value — M11b-3 §9).
    /// Direction-independent: purely "which summed higher."
    pub fn compare_labels(&self, a: &str, b: &str) -> Option<std::cmp::Ordering> {
        let sum_of = |label: &str| -> Option<i64> {
            let recs = self.by_label(label);
            if recs.is_empty() {
                return None;
            }
            Some(recs.iter().filter(|r| r.kept).map(|r| r.value as i64).sum())
        };
        let sa = sum_of(a)?;
        let sb = sum_of(b)?;
        Some(sa.cmp(&sb))
    }
}
```

Note: the "unordered" exclusion in the doc comment is aspirational until Task 6 introduces `is_ordered`; this task's implementation is already correct for it — an unordered `Faces` die's `value` field is defined to be `0` (Task 5), which is a valid `i64` to sum, so no special-casing is needed here. Re-verify this claim once Task 6 lands (it does not change `compare_labels`'s code, only what `value` means upstream).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat-server by_label_collects_only_matching_records compare_labels_orders_by_sum --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/dice/outcome.rs
git commit -m "feat(dice/m11b-3): add RollOutcome::by_label + compare_labels"
```

---

## Task 3: Label notation `[label]`

**Files:**
- Modify: `src/server/src/dice/notation/lexer.rs` (new tokens)
- Modify: `src/server/src/dice/notation/mod.rs` (new `ParseError` variants)
- Modify: `src/server/src/dice/notation/parser.rs:141-181` (`factor()` — attach a parsed label to the constructed `DiceGroup`)
- Test: `src/server/src/dice/notation/lexer.rs` and `src/server/src/dice/notation/parser.rs`'s existing test modules

**Interfaces:**
- Consumes: `DiceGroup.label` (Task 1).
- Produces: nothing new consumed by later tasks; this is a leaf notation feature.

- [ ] **Step 1: Write the failing lexer tests**

Add to `src/server/src/dice/notation/lexer.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn lex_label_brackets() {
    let toks = lex("1d12[Hope]").unwrap();
    assert_eq!(
        toks,
        vec![
            Token::Int(1),
            Token::D,
            Token::Int(12),
            Token::LBracket,
            Token::Ident("hope".to_string().to_lowercase()), // placeholder — see Step 3 note
            Token::RBracket,
        ]
    );
}
```

Actually — labels must preserve case ("Hope" must round-trip as "Hope", not "hope"; the existing `Ident` arm lowercases via `.to_lowercase()` for modifier keywords like `kh`/`cs`, which is wrong for a label's display text). Replace the test above with the real target shape before writing code — a label is lexed as a single raw-text token, not reusing `Ident`:

```rust
#[test]
fn lex_label_brackets_preserve_case() {
    let toks = lex("1d12[Hope]").unwrap();
    assert_eq!(
        toks,
        vec![
            Token::Int(1),
            Token::D,
            Token::Int(12),
            Token::Label("Hope".to_string()),
        ]
    );
}

#[test]
fn lex_label_rejects_empty() {
    assert!(matches!(lex("1d12[]"), Err(ParseError::EmptyLabel)));
}

#[test]
fn lex_label_rejects_unterminated() {
    assert!(matches!(lex("1d12[Hope"), Err(ParseError::UnterminatedLabel)));
}

#[test]
fn lex_label_trims_surrounding_whitespace() {
    let toks = lex("1d12[ Hope ]").unwrap();
    assert_eq!(toks, vec![Token::Int(1), Token::D, Token::Int(12), Token::Label("Hope".to_string())]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server lex_label --lib`
Expected: compile error — `Token::Label` and `ParseError::EmptyLabel`/`UnterminatedLabel` don't exist yet.

- [ ] **Step 3: Add the `Label` token and lex `[...]` as one unit**

In `src/server/src/dice/notation/lexer.rs`, add a variant to `Token`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Int(i32),
    D,
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Cmp(Comparator),
    Bang,
    BangBang,
    BangP,
    Label(String),
}
```

Add a new match arm in `lex` (place it alongside the other single-char-prefixed arms, e.g. right after the `'('`/`')'` arms):

```rust
            '[' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] as char != ']' {
                    j += 1;
                }
                if j >= bytes.len() {
                    return Err(ParseError::UnterminatedLabel);
                }
                let raw = input[start..j].trim();
                if raw.is_empty() {
                    return Err(ParseError::EmptyLabel);
                }
                out.push(Token::Label(raw.to_string()));
                i = j + 1; // skip past the ']'
            }
```

Note: labels are NOT lowercased (unlike `Ident`) — a label is display text (`"Hope"`), not a modifier keyword. The ASCII-only precondition already checked at the top of `lex` covers the label's charset (any ASCII byte except `]`, since the scan above stops at the first `]`).

- [ ] **Step 4: Add the two `ParseError` variants**

In `src/server/src/dice/notation/mod.rs`, add to `ParseError`:

```rust
    /// A `[...]` label was empty after trimming whitespace (e.g. `1d12[]` or `1d12[ ]`).
    EmptyLabel,
    /// A `[` was never closed by a matching `]` before the input ended.
    UnterminatedLabel,
```

- [ ] **Step 5: Run lexer tests to verify they pass**

Run: `cargo test -p shadowcat-server lex_label --lib`
Expected: PASS.

- [ ] **Step 6: Write the failing parser tests**

Add to `src/server/src/dice/notation/parser.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn parses_label_onto_dice_group() {
    let spec = parse("1d12[Hope]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
        _ => panic!("expected dice"),
    }
}

#[test]
fn parses_two_labeled_groups() {
    let spec = parse("1d12[Hope] + 1d12[Fear]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Bin { lhs, rhs, .. } => {
            match *lhs {
                Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
                _ => panic!("expected dice lhs"),
            }
            match *rhs {
                Expr::Dice(g) => assert_eq!(g.label, Some("Fear".to_string())),
                _ => panic!("expected dice rhs"),
            }
        }
        _ => panic!("expected Bin"),
    }
}

#[test]
fn duplicate_labels_across_groups_are_not_an_error() {
    // Two groups intentionally sharing a label pool under by_label — not a parse error.
    assert!(parse("1d6[Pool] + 1d6[Pool]", ParseContext::default()).is_ok());
}
```

- [ ] **Step 7: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server parses_label parses_two_labeled duplicate_labels --lib`
Expected: FAIL — `1d12[Hope]` currently errors with `ParseError::Trailing` (the `[` byte isn't consumed by `factor()`, so the token stream still has a dangling `Label`/error token after the dice factor).

- [ ] **Step 8: Wire the label into `factor()`**

In `src/server/src/dice/notation/parser.rs`, change the dice-factor branch of `factor()` from:

```rust
                    let modifiers = self.modifiers(sides)?;
                    Ok(Expr::Dice(DiceGroup {
                        count: n as u32,
                        kind: DieKind::Numeric { min: 1, max: sides },
                        modifiers,
                    }))
```

to:

```rust
                    let modifiers = self.modifiers(sides)?;
                    let label = if let Some(Token::Label(_)) = self.peek() {
                        match self.bump() {
                            Some(Token::Label(l)) => Some(l),
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    Ok(Expr::Dice(DiceGroup {
                        count: n as u32,
                        kind: DieKind::Numeric { min: 1, max: sides },
                        modifiers,
                        label,
                    }))
```

(The label is checked for AFTER modifiers, matching the design's `1d12[Hope]` placement — immediately after the group's dice + modifiers.)

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p shadowcat-server parses_label parses_two_labeled duplicate_labels --lib`
Expected: PASS.

- [ ] **Step 10: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 11: Commit**

```bash
git add src/server/src/dice/notation/lexer.rs src/server/src/dice/notation/mod.rs src/server/src/dice/notation/parser.rs
git commit -m "feat(dice/m11b-3): parse [label] notation onto DiceGroup"
```

---

## Task 4: `DieKind::Faces` data model + `Face`/`Symbol` + validation

**Files:**
- Modify: `src/server/src/dice/spec.rs:6-10` (`DieKind`)
- Test: `src/server/src/dice/spec.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `DieKind::Faces { faces: Vec<Face> }`, `Face { value: Option<i32>, symbols: Vec<Symbol> }`, `type Symbol = String`, `DieKind::validate(&self) -> Result<(), DieKindError>` (or equivalent — exact error type below) — consumed by Task 5 (RNG), Task 6 (`is_ordered`), Task 11 (recalc).

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/spec.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn faces_die_validate_rejects_empty_face_list() {
    let kind = DieKind::Faces { faces: vec![] };
    assert!(matches!(kind.validate(), Err(DieKindError::EmptyFaces)));
}

#[test]
fn faces_die_validate_accepts_nonempty_face_list() {
    let kind = DieKind::Faces {
        faces: vec![Face { value: Some(1), symbols: vec![] }],
    };
    assert!(kind.validate().is_ok());
}

#[test]
fn numeric_die_validate_is_always_ok() {
    assert!(DieKind::Numeric { min: 1, max: 6 }.validate().is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server faces_die_validate numeric_die_validate --lib`
Expected: compile error — `DieKind::Faces`, `Face`, `DieKindError`, `.validate()` don't exist yet.

- [ ] **Step 3: Implement `DieKind::Faces`, `Face`, `Symbol`, and `validate`**

In `src/server/src/dice/spec.rs`, change:

```rust
/// A die's face space. M11a: numeric only; M11b adds `Faces` for custom-symbol dice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    Numeric { min: i32, max: i32 },
}
```

to:

```rust
/// A single face of a `DieKind::Faces` die. `value` is `Some` for a face that
/// participates numerically (ordering, totals); `None` for a face whose only
/// payload is `symbols`. A `Faces` die is "ordered" (see `eval::classify` /
/// `is_ordered`, M11b-3 §9) iff EVERY face has `value: Some`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Face {
    pub value: Option<i32>,
    pub symbols: Vec<Symbol>,
}

/// An opaque tag on a `Face`; the system assigns meaning (e.g. Genesys "triumph").
pub type Symbol = String;

/// A die's face space. `Numeric`: an ordered inclusive range. `Faces`: an
/// explicit, possibly-unordered, possibly-symbolic list (M11b-3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    Numeric { min: i32, max: i32 },
    Faces { faces: Vec<Face> },
}

/// Construction-time validation error for a `DieKind`. `Numeric` has no
/// invalid state representable by this type (`sides >= 1` is enforced by the
/// notation parser's `ParseError::InvalidDieSides`, since only the notation
/// path constructs `Numeric` from untrusted input today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DieKindError {
    /// `Faces { faces: [] }` — `roll_uniform(0, faces.len() - 1)` requires a
    /// non-degenerate range; `roll_uniform` only `debug_assert!`s this (a
    /// no-op in release). No notation path constructs `Faces` today (M11b-3
    /// is struct-only for face-lists); this becomes the enforcement point at
    /// M11d's untrusted-wire boundary.
    EmptyFaces,
}

impl DieKind {
    pub fn validate(&self) -> Result<(), DieKindError> {
        match self {
            DieKind::Numeric { .. } => Ok(()),
            DieKind::Faces { faces } => {
                if faces.is_empty() {
                    Err(DieKindError::EmptyFaces)
                } else {
                    Ok(())
                }
            }
        }
    }
}
```

- [ ] **Step 4: Fix the exhaustive `DieKind::Numeric` matches this introduces**

Adding a `Faces` variant to a previously-single-variant-effectively `enum DieKind` breaks every `let DieKind::Numeric { min, max } = ...` irrefutable-pattern binding in the crate (Rust requires exhaustive patterns once a second variant exists). The current sites:

- `src/server/src/dice/eval/groups.rs:29` — `let DieKind::Numeric { min, max } = group.kind;` inside `resolve_group`
- `src/server/src/dice/eval/mod.rs:28` — `let DieKind::Numeric { min, max } = group.kind;` inside `roll_expr`
- `src/server/src/dice/eval/expertise.rs:130` — inside `allocate`'s `bounds` map closure
- `src/server/src/dice/recalc.rs:52` — inside `RecalcOp::RerollDice`'s per-die loop

Leave these as todo markers for Task 5/Task 6/Task 10/Task 11 respectively — those tasks change this exact code to branch on both variants with real behavior. For THIS task, only make the crate compile again without changing behavior: change each irrefutable `let DieKind::Numeric { min, max } = X;` to `let (min, max) = match X { DieKind::Numeric { min, max } => (min, max), DieKind::Faces { .. } => unreachable!("Faces not yet reachable from this path — M11b-3 Task N wires it") };` — EXCEPT do not add this scaffolding if the later task in this same plan already rewrites that exact line (it does, for all four sites above). Instead: **for this task, do nothing to those four call sites** — they do not yet fail to compile, because nothing in the crate can construct a `DiceGroup`/`RawDie` with `DieKind::Faces` yet (no notation, no RNG path reaches it). Confirm this by running the build:

Run: `cargo build --tests -p shadowcat-server`
Expected: clean build. (The four sites above pattern-match on a *value* of type `DieKind`, not the type itself in an exhaustive-match position that the compiler checks at this point — `let DieKind::Numeric { .. } = group.kind` is only rejected by the compiler as non-exhaustive once it actually IS non-exhaustive for a *reachable* value, which Rust checks structurally against the enum definition, not reachability. If this build step reports "refutable pattern" errors, fix each reported site with the `match` form shown above, then proceed — but expect zero, since Rust's exhaustiveness check fires on the enum's variant count, not runtime reachability.)

**Correction to the above note:** Rust's `let PATTERN = EXPR;` exhaustiveness check IS structural (checks the enum's variant list, not runtime reachability) — so all four sites WILL fail to compile the moment `DieKind` gains a second variant. Do not skip this: apply the `match` rewrite to all four sites now, each producing an `unreachable!()` arm on `Faces` (later tasks replace the `unreachable!()` with real logic, each in its own task/commit):

`groups.rs:29`:
```rust
    let (min, max) = match group.kind {
        DieKind::Numeric { min, max } => (min, max),
        DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into resolve_group (M11b-3 Task 6)"),
    };
```
(then replace every subsequent bare `min`/`max` use in the function body — none needed, they're already local bindings of the same names, so no further edits in this function.)

`eval/mod.rs:28` (inside `roll_expr`):
```rust
            let (min, max) = match group.kind {
                DieKind::Numeric { min, max } => (min, max),
                DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into roll_expr (M11b-3 Task 5)"),
            };
```

`eval/expertise.rs:130` (inside `allocate`'s bounds map closure):
```rust
        .map(|d| {
            let (min, max) = match d.kind {
                DieKind::Numeric { min, max } => (min, max),
                DieKind::Faces { .. } => unreachable!("Faces dice excluded from expertise (M11b-3 Task 10)"),
            };
            (d.id, (min, max))
        })
```

`recalc.rs:52` (inside `RecalcOp::RerollDice`'s loop):
```rust
                        if ids.contains(&d.id) {
                            let (min, max) = match d.kind {
                                DieKind::Numeric { min, max } => (min, max),
                                DieKind::Faces { .. } => unreachable!("Faces recalc not yet wired (M11b-3 Task 11)"),
                            };
                            d.natural = roll_uniform(rng, min, max);
                        }
```

- [ ] **Step 5: Run test to verify it passes, and re-run the full suite**

Run: `cargo test -p shadowcat-server faces_die_validate numeric_die_validate --lib`
Expected: PASS.

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS (the four `unreachable!()` arms are never hit yet — nothing constructs `Faces`).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/dice/spec.rs src/server/src/dice/eval/groups.rs src/server/src/dice/eval/mod.rs src/server/src/dice/eval/expertise.rs src/server/src/dice/recalc.rs
git commit -m "feat(dice/m11b-3): add DieKind::Faces + Face/Symbol + validate()"
```

---

## Task 5: Custom-face RNG draw + `value`/`symbols` derivation

**Files:**
- Modify: `src/server/src/dice/eval/mod.rs:25-48` (`roll_expr`)
- Modify: `src/server/src/dice/eval/groups.rs:22-44` (`resolve_group`'s initial map — derive `value`/`symbols` from the index)
- Modify: `src/server/src/dice/outcome.rs:49-67` (`DieRecord` gains `symbols: Vec<Symbol>`) — mechanical compile-fix on the same 4 sites Task 1 already touched for `label` (add `symbols: vec![],` alongside)
- Test: `src/server/src/dice/eval/groups.rs`, `src/server/src/dice/rng.rs`

**Interfaces:**
- Consumes: `DieKind::Faces`/`Face`/`Symbol` (Task 4).
- Produces: `DieRecord.symbols: Vec<Symbol>` — consumed by Task 6 (`is_ordered` skip logic reads `Face.value`, not this field, but downstream success/crit predicates in Tasks 7-8 read `DieRecord.symbols`), Task 9 (`symbol_counts`).

- [ ] **Step 1: Write the failing tests**

Add to `src/server/src/dice/eval/groups.rs`'s `#[cfg(test)] mod tests`:

```rust
fn faces_group(faces: Vec<crate::dice::spec::Face>, count: u32) -> DiceGroup {
    DiceGroup {
        count,
        kind: DieKind::Faces { faces },
        modifiers: vec![],
        label: None,
    }
}

#[test]
fn faces_die_derives_value_and_symbols_from_index() {
    use crate::dice::spec::Face;
    let faces = vec![
        Face { value: Some(1), symbols: vec!["blank".into()] },
        Face { value: Some(3), symbols: vec!["success".into(), "triumph".into()] },
    ];
    // natural = 1 selects faces[1].
    let naturals = vec![RawDie { id: 0, kind: DieKind::Faces { faces: faces.clone() }, natural: 1 }];
    let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 1, group_spans: vec![] };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&faces_group(faces, 1), 0, &naturals, &mut rng, &mut raws);
    assert_eq!(recs[0].value, 3);
    assert_eq!(recs[0].symbols, vec!["success".to_string(), "triumph".to_string()]);
}

#[test]
fn faces_die_none_value_face_contributes_zero() {
    use crate::dice::spec::Face;
    let faces = vec![Face { value: None, symbols: vec!["blank".into()] }];
    let naturals = vec![RawDie { id: 0, kind: DieKind::Faces { faces: faces.clone() }, natural: 0 }];
    let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 1, group_spans: vec![] };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&faces_group(faces, 1), 0, &naturals, &mut rng, &mut raws);
    assert_eq!(recs[0].value, 0, "a None-value face contributes 0 numerically");
    assert_eq!(recs[0].symbols, vec!["blank".to_string()]);
}
```

Add to `src/server/src/dice/rng.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn roll_uniform_over_face_index_range_stays_in_bounds() {
    // A 3-face die draws an index in 0..=2 via the same roll_uniform used for Numeric.
    let mut r = NoiseRng::from_seed(3);
    for _ in 0..500 {
        let idx = roll_uniform(&mut r, 0, 2);
        assert!((0..=2).contains(&idx));
    }
}
```

(This last test exercises existing `roll_uniform` over a 0-based range — it does not require new RNG code, but pins the exact call shape Task 5's `roll_expr` change will use, so it's written first per TDD.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server faces_die_derives faces_die_none_value roll_uniform_over_face_index --lib`
Expected: FAIL — `resolve_group`'s current initial map only reads `d.natural` as a numeric value directly (`value: d.natural`), so a `Faces` die's `natural` (an index) is used raw instead of looked up; also `DieRecord.symbols` doesn't exist yet (compile error).

- [ ] **Step 3: Add `symbols` to `DieRecord`**

In `src/server/src/dice/outcome.rs`, add after the `label` field added in Task 1:

```rust
    /// Resolved symbols for a `Faces` die's drawn face (M11b-3); empty for `Numeric`.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
```

Add the import: `use crate::dice::spec::{DieId, DieKind, RollSpec, Symbol};` (extend the existing `use` line).

Mechanically fix the same 4 sites Task 1 fixed for `label` (add `symbols: vec![],` alongside `label: None,`): the two production sites in `groups.rs` (Step 4 below handles these with real derivation, not a bare `vec![]`) and the two test-helper sites in `expertise.rs`/`success.rs` (bare `symbols: vec![],` — those helpers only construct `Numeric` dice, which never carry symbols).

- [ ] **Step 4: Derive `value`/`symbols` from a `Faces` die's index in `resolve_group`**

In `src/server/src/dice/eval/groups.rs`, change the initial per-natural map. Task 4 Step 4 already replaced the original `let DieKind::Numeric { min, max } = group.kind;` line at the top of `resolve_group` with a `match`-producing-`unreachable!()` placeholder — LEAVE that line exactly as Task 4 left it (the `Reroll`/`Explode` arms further down the function still read `min`/`max` from it until Task 6 replaces it with a `redraw` closure). Change only the `.map()` closure that builds the initial `recs`, from the CURRENT state (post-Task-4):

```rust
    let (min, max) = match group.kind {
        DieKind::Numeric { min, max } => (min, max),
        DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into resolve_group (M11b-3 Task 6)"),
    };
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| DieRecord {
            id: d.id,
            group_index,
            natural: d.natural,
            value: d.natural,
            kept: true,
            exploded: false,
            rerolled_from: None,
            crit_success: false,
            crit_fail: false,
            expertise: 0,
            label: group.label.clone(),
        })
        .collect();
```

to:

```rust
    let (min, max) = match group.kind {
        DieKind::Numeric { min, max } => (min, max),
        DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into resolve_group (M11b-3 Task 6)"),
    };
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| {
            let (value, symbols) = face_value_and_symbols(&group.kind, d.natural);
            DieRecord {
                id: d.id,
                group_index,
                natural: d.natural,
                value,
                kept: true,
                exploded: false,
                rerolled_from: None,
                crit_success: false,
                crit_fail: false,
                expertise: 0,
                label: group.label.clone(),
                symbols,
            }
        })
        .collect();
```

The rest of `resolve_group` (reroll/explode/keep-drop) still reads `group.kind` for `min`/`max` in the `Reroll`/`Explode` match arms — those arms are gated to `Numeric`-only in **Task 6**, not here; for this task, leave them referencing `group.kind` as-is (they will not be reached for a `Faces` group because Task 6 adds the `is_ordered` skip before those modifiers run — until Task 6 lands, a `Faces` group with a `Reroll`/`Explode` modifier is simply not exercised by any test in this task).

Add a helper function (place it near the top of `groups.rs`, above `resolve_group`):

```rust
/// Look up a die's post-index value/symbols. `Numeric` dice pass their natural
/// straight through (no faces table). A `Faces` die's `natural` is a face
/// INDEX; a `None`-value face contributes `0` numerically.
fn face_value_and_symbols(kind: &DieKind, natural: i32) -> (i32, Vec<crate::dice::spec::Symbol>) {
    match kind {
        DieKind::Numeric { .. } => (natural, vec![]),
        DieKind::Faces { faces } => {
            let face = &faces[natural as usize];
            (face.value.unwrap_or(0), face.symbols.clone())
        }
    }
}
```

`push_extra`'s own `DieRecord` literal (not just its callers) needs a `symbols` field now that `DieRecord` has one — without this the crate will not compile. Change `push_extra` from:

```rust
fn push_extra(
    recs: &mut Vec<DieRecord>,
    raws: &mut RawRoll,
    kind: DieKind,
    group_index: usize,
    label: Option<String>,
    natural: i32,
    value: i32,
) {
    let id = raws.push(kind, natural);
    recs.push(DieRecord {
        id,
        group_index,
        natural,
        value,
        kept: true,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label,
    });
}
```

to (add `symbols: vec![]` — `push_extra` is called only from the `Standard`/`Penetrate` `Explode` arms, both `Numeric`-only paths until Task 6 reworks explode for `Faces` dice, so an empty vec is exact here, not a placeholder):

```rust
fn push_extra(
    recs: &mut Vec<DieRecord>,
    raws: &mut RawRoll,
    kind: DieKind,
    group_index: usize,
    label: Option<String>,
    natural: i32,
    value: i32,
) {
    let id = raws.push(kind, natural);
    recs.push(DieRecord {
        id,
        group_index,
        natural,
        value,
        kept: true,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label,
        symbols: vec![],
    });
}
```

No change is needed at `push_extra`'s two call sites in the `Explode` match arm for this task — they are `Numeric`-only paths (a `Faces` die never reaches `Explode` after Task 6's gate).

- [ ] **Step 5: Wire the RNG draw in `roll_expr` (`eval/mod.rs`)**

Task 4 Step 4 already replaced this function's original single-line binding with a `match`-producing-`unreachable!()` placeholder. Change (the CURRENT state, post-Task-4):

```rust
        Expr::Dice(group) => {
            let (min, max) = match group.kind {
                DieKind::Numeric { min, max } => (min, max),
                DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into roll_expr (M11b-3 Task 5)"),
            };
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = roll_uniform(rng, min, max);
                raws.push(group.kind.clone(), natural);
            }
```

to:

```rust
        Expr::Dice(group) => {
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = match &group.kind {
                    DieKind::Numeric { min, max } => roll_uniform(rng, *min, *max),
                    DieKind::Faces { faces } => roll_uniform(rng, 0, faces.len() as i32 - 1),
                };
                raws.push(group.kind.clone(), natural);
            }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p shadowcat-server faces_die_derives faces_die_none_value roll_uniform_over_face_index --lib`
Expected: PASS.

- [ ] **Step 7: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/dice/eval/groups.rs src/server/src/dice/eval/mod.rs src/server/src/dice/outcome.rs
git commit -m "feat(dice/m11b-3): draw Faces dice by index, derive value/symbols"
```

---

## Task 6: `is_ordered` predicate — gate every value-reading op

**Files:**
- Modify: `src/server/src/dice/spec.rs` (`impl DieKind` — add `is_ordered`)
- Modify: `src/server/src/dice/eval/groups.rs` (gate `Reroll`/`Explode`/keep-drop to ordered dice)
- Modify: `src/server/src/dice/eval/sum.rs:35-66` (`fold` — an unordered die contributes 0)
- Test: `src/server/src/dice/spec.rs`, `src/server/src/dice/eval/groups.rs`, `src/server/src/dice/eval/sum.rs`

**Interfaces:**
- Consumes: `DieKind::Faces`/`Face` (Task 4).
- Produces: `DieKind::is_ordered(&self) -> bool` — consumed by Task 7 (success predicate doesn't need it directly — `HasSymbol` works regardless of ordering — but Task 10's expertise guard reads `is_ordered` transitively via the `Numeric`-only filter).

- [ ] **Step 1: Write the failing `is_ordered` unit tests**

Add to `src/server/src/dice/spec.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn numeric_is_always_ordered() {
    assert!(DieKind::Numeric { min: 1, max: 6 }.is_ordered());
}

#[test]
fn faces_all_valued_is_ordered() {
    let kind = DieKind::Faces {
        faces: vec![
            Face { value: Some(1), symbols: vec![] },
            Face { value: Some(2), symbols: vec!["x".into()] },
        ],
    };
    assert!(kind.is_ordered());
}

#[test]
fn faces_any_none_value_is_unordered() {
    let kind = DieKind::Faces {
        faces: vec![
            Face { value: Some(1), symbols: vec![] },
            Face { value: None, symbols: vec!["blank".into()] },
        ],
    };
    assert!(!kind.is_ordered());
}

#[test]
fn faces_all_none_value_is_unordered() {
    let kind = DieKind::Faces {
        faces: vec![Face { value: None, symbols: vec!["x".into()] }],
    };
    assert!(!kind.is_ordered());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server numeric_is_always_ordered faces_all_valued_is_ordered faces_any_none faces_all_none --lib`
Expected: compile error — `is_ordered` doesn't exist.

- [ ] **Step 3: Implement `is_ordered`**

In `src/server/src/dice/spec.rs`, extend the `impl DieKind` block added in Task 4:

```rust
impl DieKind {
    pub fn validate(&self) -> Result<(), DieKindError> { /* unchanged from Task 4 */ }

    /// A die participates in value-based operations (fold-into-total, keep/drop,
    /// comparator explode/reroll) iff its faces have a defined ordering.
    /// `Numeric` is always ordered. `Faces` is ordered iff EVERY face has
    /// `value: Some` — a single unordered face makes the whole die unrankable
    /// against a valued sibling (M11b-3 §9/design decision).
    pub fn is_ordered(&self) -> bool {
        match self {
            DieKind::Numeric { .. } => true,
            DieKind::Faces { faces } => faces.iter().all(|f| f.value.is_some()),
        }
    }
}
```

- [ ] **Step 4: Run `is_ordered` tests to verify they pass**

Run: `cargo test -p shadowcat-server numeric_is_always_ordered faces_all_valued_is_ordered faces_any_none faces_all_none --lib`
Expected: PASS.

- [ ] **Step 5: Write the failing gate tests in `groups.rs`**

Add to `src/server/src/dice/eval/groups.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn unordered_faces_die_is_skipped_by_keep_highest() {
    use crate::dice::spec::Face;
    // Two unordered faces dice (value: None) mixed conceptually with keep-highest —
    // since ALL dice in this group are the same DieKind, this test exercises the
    // group-level gate: an unordered Faces group's keep/drop modifier must be a
    // no-op (every die stays kept), not a ranking-by-value(0) accident.
    let faces = vec![
        Face { value: None, symbols: vec!["a".into()] },
        Face { value: None, symbols: vec!["b".into()] },
    ];
    let naturals = vec![
        RawDie { id: 0, kind: DieKind::Faces { faces: faces.clone() }, natural: 0 },
        RawDie { id: 1, kind: DieKind::Faces { faces: faces.clone() }, natural: 1 },
    ];
    let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 2, group_spans: vec![] };
    let g = DiceGroup {
        count: 2,
        kind: DieKind::Faces { faces },
        modifiers: vec![GroupModifier::KeepHighest(1)],
        label: None,
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    assert!(recs.iter().all(|r| r.kept), "unordered Faces group must skip keep/drop entirely");
}

#[test]
fn ordered_faces_die_participates_in_keep_highest_like_numeric() {
    use crate::dice::spec::Face;
    let faces = vec![
        Face { value: Some(1), symbols: vec![] },
        Face { value: Some(6), symbols: vec![] },
    ];
    let naturals = vec![
        RawDie { id: 0, kind: DieKind::Faces { faces: faces.clone() }, natural: 0 }, // value 1
        RawDie { id: 1, kind: DieKind::Faces { faces: faces.clone() }, natural: 1 }, // value 6
    ];
    let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 2, group_spans: vec![] };
    let g = DiceGroup {
        count: 2,
        kind: DieKind::Faces { faces },
        modifiers: vec![GroupModifier::KeepHighest(1)],
        label: None,
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    let kept: Vec<i32> = recs.iter().filter(|r| r.kept).map(|r| r.value).collect();
    assert_eq!(kept, vec![6], "ordered Faces group ranks exactly like Numeric");
}
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server unordered_faces_die_is_skipped ordered_faces_die_participates --lib`
Expected: FAIL — `KeepHighest`/`KeepLowest`/`DropHighest`/`DropLowest` currently apply unconditionally (`keep(&mut recs, *n as usize, ..)`), ranking by `value` regardless of `is_ordered`; for the unordered case the die at `value: 0`/`value: 0` (both `None`-value faces collapse to `0`) ties and one is arbitrarily dropped rather than both staying kept.

- [ ] **Step 7: Gate `resolve_group`'s modifier loop on `is_ordered`**

In `src/server/src/dice/eval/groups.rs`, wrap the modifier `match` inside the `for m in &group.modifiers` loop with an early skip when the group is unordered, EXCEPT that `Reroll`/`Explode` need the same gate (comparator triggers read `value`, meaningless when unordered). Change:

```rust
    for m in &group.modifiers {
        match m {
            GroupModifier::Reroll { comp, target, once } => {
```

to:

```rust
    let ordered = group.kind.is_ordered();
    for m in &group.modifiers {
        if !ordered {
            // Unordered Faces dice have no rankable value — every value-reading
            // modifier (reroll/explode-by-comparator, keep/drop) is a no-op.
            continue;
        }
        match m {
            GroupModifier::Reroll { comp, target, once } => {
```

(The `let DieKind::Numeric { min, max } = group.kind;` — already converted to a `match` producing `(min,max)` is NOT what we have here; re-check: Task 4 Step 4 already converted `groups.rs:29`'s binding into an `unreachable!()`-on-Faces match. That binding is used inside the `Reroll`/`Explode` arms for `min`/`max`. Since those arms are now unreachable for `Faces` (gated above by `if !ordered { continue; }` before the match even runs for a Faces group's modifiers) ONLY IF a `Faces` group is fully unordered; an ORDERED `Faces` group still reaches the `Reroll`/`Explode` arms and calls `roll_uniform(rng, min, max)` — but an ordered `Faces` die has no `(min,max)` range, only a face-index space. Resolve this: an ordered `Faces` die's `Reroll`/`Explode` must redraw a fresh **index**, not a numeric range. Fix the `unreachable!()` placeholder from Task 4 now:)

Replace the Task-4 placeholder at the top of `resolve_group`:

```rust
    let (min, max) = match group.kind {
        DieKind::Numeric { min, max } => (min, max),
        DieKind::Faces { .. } => unreachable!("Faces dice not yet wired into resolve_group (M11b-3 Task 6)"),
    };
```

with a redraw closure usable by both variants:

```rust
    let redraw = |rng: &mut dyn RngSource| -> i32 {
        match &group.kind {
            DieKind::Numeric { min, max } => roll_uniform(rng, *min, *max),
            DieKind::Faces { faces } => roll_uniform(rng, 0, faces.len() as i32 - 1),
        }
    };
```

and replace every `roll_uniform(rng, min, max)` call inside the `Reroll`/`Explode` arms of this function with `redraw(rng)`. There are three such call sites in the current file: the `Reroll` arm's `r.value = roll_uniform(rng, min, max);`, and the `Explode` arm's `let extra = roll_uniform(rng, min, max);`. Additionally, since a redrawn value for a `Faces` die is an INDEX (not the final numeric value), the `Reroll`/`Explode` arms must also re-derive `(value, symbols)` via `face_value_and_symbols(&group.kind, extra)` (Task 5's helper) rather than assigning the raw draw directly. Apply this change:

Reroll arm — change:
```rust
                    while comp.test(r.value, *target) && chain < CHAIN_CAP {
                        r.rerolled_from = Some(r.value);
                        r.value = roll_uniform(rng, min, max);
                        chain += 1;
                        if *once {
                            break;
                        }
                    }
```
to:
```rust
                    while comp.test(r.value, *target) && chain < CHAIN_CAP {
                        r.rerolled_from = Some(r.value);
                        let drawn = redraw(rng);
                        let (value, symbols) = face_value_and_symbols(&group.kind, drawn);
                        r.value = value;
                        r.symbols = symbols;
                        chain += 1;
                        if *once {
                            break;
                        }
                    }
```

Explode arm — change the `let extra = roll_uniform(rng, min, max);` line to `let extra = redraw(rng);`, and change the `ExplodeKind::Compound => { recs[i].value += extra; }` arm to first derive `(value, symbols)` from `extra` via `face_value_and_symbols` before combining — for `Compound` on a `Faces` die there is no well-defined "add an index to a value," so restrict `Compound`/`Penetrate` explosion to `Numeric` only: wrap those two arms' bodies in a check that `matches!(group.kind, DieKind::Numeric { .. })`, and for a `Faces` die route ALL explode kinds through the `Standard` push-a-new-die path (append a new `Faces` die at the redrawn index) instead. Concretely:

```rust
                            match kind {
                                ExplodeKind::Compound if matches!(group.kind, DieKind::Numeric { .. }) => {
                                    recs[i].value += extra;
                                }
                                ExplodeKind::Penetrate if matches!(group.kind, DieKind::Numeric { .. }) => {
                                    let value = extra - 1;
                                    push_extra(&mut recs, raws, group.kind.clone(), group_index, group.label.clone(), extra, value);
                                }
                                _ => {
                                    // Standard explode (or Compound/Penetrate on an
                                    // ordered Faces die, where "add"/"−1" have no
                                    // defined meaning): push a fresh die at the drawn index.
                                    let (value, symbols) = face_value_and_symbols(&group.kind, extra);
                                    let id = raws.push(group.kind.clone(), extra);
                                    recs.push(DieRecord {
                                        id,
                                        group_index,
                                        natural: extra,
                                        value,
                                        kept: true,
                                        exploded: false,
                                        rerolled_from: None,
                                        crit_success: false,
                                        crit_fail: false,
                                        expertise: 0,
                                        label: group.label.clone(),
                                        symbols,
                                    });
                                }
                            }
```

This subsumes `push_extra` for the fallback arm (inlined since it now needs `symbols` derivation `push_extra` doesn't do — alternatively, extend `push_extra` to take `Vec<Symbol>` and call `face_value_and_symbols` internally; either is acceptable, but keep `push_extra`'s signature single-purpose and do the inline form shown above to avoid re-threading yet another parameter through the `Numeric`-only Penetrate call). Re-run the existing Penetrate/Standard explode tests from `groups.rs` (`standard_explode_appends_extra_on_max`, `penetrate_retrigger_uses_raw_face_not_decremented_value`, etc.) after this change to confirm the `Numeric` behavior is byte-for-byte unchanged (the `matches!(group.kind, DieKind::Numeric { .. })` guard preserves the original arms exactly for `Numeric`).

Finally, update the `keep` calls at the bottom of the modifier match — no change needed to `fn keep` itself (it already ranks by `.value`, which is now well-defined as `0` for every unordered die, but the `if !ordered { continue; }` guard added above means `keep` is simply never called for an unordered group, so its ranking-by-0 behavior is unreachable dead code for that case — leave `fn keep` as-is).

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p shadowcat-server unordered_faces_die_is_skipped ordered_faces_die_participates --lib`
Expected: PASS.

Run: `cargo test -p shadowcat-server dice::eval::groups --lib`
Expected: all PASS, including every pre-existing `Numeric` explode/reroll/keep-drop test (behavior byte-for-byte unchanged for `Numeric`).

- [ ] **Step 9: Write the failing `fold` test in `sum.rs`**

Add to `src/server/src/dice/eval/sum.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn unordered_faces_die_contributes_zero_to_total() {
    use crate::dice::spec::Face;
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup {
            count: 1,
            kind: DieKind::Faces {
                faces: vec![Face { value: None, symbols: vec!["x".into()] }],
            },
            modifiers: vec![],
            label: None,
        }),
        direction: Direction::HighWins,
        mode: total_mode(),
    };
    let raws = roll(&spec, &mut NoiseRng::from_seed(1));
    let out = evaluate(&spec, &raws);
    assert_eq!(out.total, 0);
}
```

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test -p shadowcat-server unordered_faces_die_contributes_zero --lib`
Expected: PASS already, actually — Task 5's `face_value_and_symbols` already makes a `None`-value face's `value` field `0`, and `fold`'s `Expr::Dice(_)` arm already sums `r.value` unconditionally. Confirm this by running the test BEFORE making any change in this step: if it passes, no code change is needed for `fold` — Task 5 already satisfies the "contributes 0" requirement, because an unordered die's `value` IS `0` by construction, not because `fold` special-cases orderedness. Record this finding and move to Step 11 without modifying `sum.rs`'s `fold` function.

- [ ] **Step 11: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 12: Commit**

```bash
git add src/server/src/dice/spec.rs src/server/src/dice/eval/groups.rs src/server/src/dice/eval/sum.rs
git commit -m "feat(dice/m11b-3): gate keep/drop/explode/reroll on DieKind::is_ordered"
```

---

## Task 7: `SuccessRule` becomes an enum (`Numeric` default / `HasSymbol`)

**Files:**
- Modify: `src/server/src/dice/spec.rs` (`SuccessRule`, `Comparator` gains `Default`)
- Modify: `src/server/src/dice/eval/success.rs:27` (the `cfg.success.comp.test(...)` call site)
- Modify: `src/server/src/dice/eval/expertise.rs:41` (`die_values`'s `cfg.success.comp.test(...)` call site)
- Modify: `src/server/src/dice/notation/parser.rs:51-57` (the `t_target`-derived `SuccessRule` construction — only site building a `SuccessRule` from parsed input)
- Modify (mechanical): every other `SuccessRule { comp, target }` literal (test-only) — wrap as `SuccessRule::Numeric { comp, target }` (list below)
- Modify (mechanical): `src/server/src/dice/proptests.rs:224` — `c.success.target` becomes a `match`
- Test: `src/server/src/dice/spec.rs`

**Interfaces:**
- Produces: `SuccessRule::Numeric { comp, target }` / `SuccessRule::HasSymbol(Symbol)` — consumed by Task 8 (crit path reuses the same enum-match idiom), and by `success.rs`/`expertise.rs`'s per-die success test.
- Consumes: `DieRecord.symbols` (Task 5).

- [ ] **Step 1: Write the failing tests**

Add to `src/server/src/dice/spec.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn success_rule_defaults_to_numeric() {
    assert_eq!(SuccessRule::default(), SuccessRule::Numeric { comp: Comparator::Gte, target: 0 });
}

#[test]
fn comparator_defaults_to_gte() {
    assert_eq!(Comparator::default(), Comparator::Gte);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server success_rule_defaults_to_numeric comparator_defaults_to_gte --lib`
Expected: compile error — neither `SuccessRule` nor `Comparator` derives `Default` yet, and `SuccessRule` is a struct, not an enum with a `Numeric` variant.

- [ ] **Step 3: Give `Comparator` a `Default`**

In `src/server/src/dice/spec.rs`, change:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparator {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
}
```

to:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Comparator {
    Eq,
    Ne,
    Gt,
    Lt,
    #[default]
    Gte,
    Lte,
}
```

- [ ] **Step 4: Convert `SuccessRule` to an enum**

Change:

```rust
/// SuccessCount dimension 1: the per-die target a die must satisfy to score a success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessRule {
    pub comp: Comparator,
    pub target: i32,
}
```

to:

```rust
/// SuccessCount dimension 1: the per-die predicate a die must satisfy to score
/// a success. Defaults to `Numeric` (comp: Gte, target: 0) so any `Default`- or
/// serde-defaulted `SuccessConfig` never silently becomes symbol-driven.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SuccessRule {
    #[default]
    Numeric { comp: Comparator, target: i32 },
    HasSymbol(Symbol),
}
```

- [ ] **Step 5: Run the new tests to verify they pass**

Run: `cargo test -p shadowcat-server success_rule_defaults_to_numeric comparator_defaults_to_gte --lib`
Expected: PASS.

- [ ] **Step 6: Fix the two production per-die test call sites**

In `src/server/src/dice/eval/success.rs`, change:

```rust
        if cfg.success.comp.test(r.value, cfg.success.target) {
            base += 1;
        }
```

to:

```rust
        let base_success = match &cfg.success {
            SuccessRule::Numeric { comp, target } => comp.test(r.value, *target),
            SuccessRule::HasSymbol(s) => r.symbols.contains(s),
        };
        if base_success {
            base += 1;
        }
```

Add `SuccessRule` to `success.rs`'s existing `use crate::dice::spec::{RollSpec, SuccessConfig};` import.

In `src/server/src/dice/eval/expertise.rs`, change (inside `die_values`):

```rust
            let base = i32::from(cfg.success.comp.test(f, cfg.success.target));
```

to:

```rust
            let base_success = match &cfg.success {
                SuccessRule::Numeric { comp, target } => comp.test(f, *target),
                // A symbol-driven success rule is invariant under expertise's
                // face-move (moving a NUMERIC face never changes which symbols
                // a die carries — expertise only ever adjusts Numeric dice, see
                // Task 10 — so a HasSymbol rule always tests the die's fixed
                // symbol set, unaffected by k).
                SuccessRule::HasSymbol(_) => false,
            };
            let base = i32::from(base_success);
```

Add `SuccessRule` to `expertise.rs`'s imports (`use crate::dice::spec::{DieId, DieKind, Direction, SuccessConfig, SuccessRule};`).

Note the `SuccessRule::HasSymbol(_) => false` arm: `die_values` is only ever called on the dice `expertise::allocate` selects as contributing (Task 10 restricts that set to `Numeric` dice). A `HasSymbol` rule combined with an all-`Numeric` contributing set means base success is always numerically `false` for every `k` under a symbol rule, which is vacuously correct (a `Numeric` die never carries symbols, so `HasSymbol` never fires for it) — this arm exists only so the match is exhaustive, not because it's a meaningfully exercised path. Re-verify this reasoning holds once Task 10 lands.

- [ ] **Step 7: Fix the notation parser's `t_target`-derived `SuccessRule` construction**

In `src/server/src/dice/notation/parser.rs`, change:

```rust
            (None, Some(t)) => SuccessRule {
                comp: match ctx.direction {
                    Direction::HighWins => Comparator::Gte,
                    Direction::LowWins => Comparator::Lte,
                },
                target: t,
            },
```

to:

```rust
            (None, Some(t)) => SuccessRule::Numeric {
                comp: match ctx.direction {
                    Direction::HighWins => Comparator::Gte,
                    Direction::LowWins => Comparator::Lte,
                },
                target: t,
            },
```

Change the two other production `SuccessRule` constructions in the same file — the `"cs"` arm's `self.success = Some(SuccessRule { comp, target });` and the `"cf"` arm's `self.success = Some(SuccessRule { comp: invert(comp), target });` — to `SuccessRule::Numeric { comp, target }` and `SuccessRule::Numeric { comp: invert(comp), target }` respectively.

- [ ] **Step 8: Fix every remaining compile site (mechanical)**

Wrap each of the following `SuccessRule { comp, target }`/`SuccessRule { comp: X, target: Y }` test-only literals as `SuccessRule::Numeric { comp, target }` (same fields, enum-qualified):

- `src/server/src/dice/eval/crit.rs:56` (the `cfg()` test helper)
- `src/server/src/dice/eval/expertise.rs:212,255,373,522` (test helpers/cases)
- `src/server/src/dice/eval/success.rs:99,124,167,266,337,362,405,439,475,503` (test helpers/cases)
- `src/server/src/dice/notation/parser.rs:328,436,454` (test assertions matching the parsed `cfg.success` value — these compare against a `SuccessRule` value, so the expected side becomes `SuccessRule::Numeric { comp: .., target: .. }`)
- `src/server/src/dice/recalc.rs:183` (the `pool()` test helper)
- `src/server/src/dice/spec.rs:227` (the `success_config_serde_round_trips` test)

Fix `src/server/src/dice/proptests.rs`'s two `SuccessRule { .. }` construction sites (lines 20, 47) the same way, PLUS fix line 224's field access, which changes from:

```rust
            Mode::SuccessCount(c) => prop_assert_eq!(c.success.target, target),
```

to:

```rust
            Mode::SuccessCount(c) => match c.success {
                SuccessRule::Numeric { target: t, .. } => prop_assert_eq!(t, target),
                SuccessRule::HasSymbol(_) => prop_assert!(false, "expected a Numeric success rule"),
            },
```

Also fix `proptests.rs`'s two other `SuccessRule { comp, target }` construction sites at lines 149/180 (inside `direction_flip_mirrors_success_count`) the same wrapping way.

Run `cargo build --tests -p shadowcat-server` and fix any remaining "expected enum `SuccessRule`, found struct" or "no variant named" errors at their reported locations until the build is clean.

- [ ] **Step 9: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS — this task changes `SuccessRule`'s shape but not its numeric behavior, so every pre-existing assertion holds unchanged.

- [ ] **Step 10: Commit**

```bash
git add src/server/src/dice/spec.rs src/server/src/dice/eval/success.rs src/server/src/dice/eval/expertise.rs src/server/src/dice/eval/crit.rs src/server/src/dice/notation/parser.rs src/server/src/dice/recalc.rs src/server/src/dice/proptests.rs
git commit -m "feat(dice/m11b-3): SuccessRule -> enum {Numeric (default), HasSymbol}"
```

---

## Task 8: Symbol success + `HasSymbol` end-to-end test (before touching crits)

**Files:**
- Test: `src/server/src/dice/eval/success.rs`

**Interfaces:**
- Consumes: `SuccessRule::HasSymbol` (Task 7), `DieRecord.symbols` (Task 5).
- Produces: nothing new — this task is a pure verification checkpoint before the higher-risk crit-path task, confirming the base (non-crit) symbol success path works through the full `evaluate` pipeline first.

- [ ] **Step 1: Write the test**

Add to `src/server/src/dice/eval/success.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn has_symbol_success_rule_feeds_net_successes_through_evaluate() {
    use crate::dice::spec::Face;
    // 3 dice, each a 2-face symbolic die: face 0 = "blank", face 1 = "triumph".
    // Success rule: HasSymbol("triumph"). Force naturals 1,1,0 (2 triumphs, 1 blank).
    let faces = vec![
        Face { value: Some(0), symbols: vec!["blank".into()] },
        Face { value: Some(1), symbols: vec!["triumph".into()] },
    ];
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup {
            count: 3,
            kind: DieKind::Faces { faces: faces.clone() },
            modifiers: vec![],
            label: None,
        }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule::HasSymbol("triumph".to_string()),
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
            expertise: 0,
        }),
    };
    // Build raws directly (bypassing RNG) with naturals selecting faces 1,1,0.
    let mut raws = RawRoll::default();
    for &idx in &[1i32, 1, 0] {
        raws.push(DieKind::Faces { faces: faces.clone() }, idx);
    }
    let recs = crate::dice::eval::groups::resolve_group(
        match &spec.expr { Expr::Dice(g) => g, _ => unreachable!() },
        0,
        &raws.dice.clone(),
        &mut NoiseRng::from_seed(1),
        &mut raws,
    );
    raws.records = recs;
    raws.group_spans = vec![(0, 3)];
    let out = evaluate(&spec, &raws);
    assert_eq!(out.successes, Some(2), "2 of 3 dice show the triumph symbol");
}
```

Add `RawRoll` to `success.rs`'s test-module imports if not already present (it is, via `crate::dice::outcome::DieRecord` — add `RawRoll` alongside).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server has_symbol_success_rule_feeds_net_successes --lib`
Expected: this SHOULD already pass if Tasks 1-7 are correctly implemented (no new production code in this task) — if it fails, that means one of Tasks 5-7's implementations has a defect. Treat a failure here as a signal to re-examine Task 5 (`face_value_and_symbols`)/Task 7 (the `HasSymbol` match arm in `success.rs`) before proceeding, not as expected red state to "fix" with new code in this task.

- [ ] **Step 3: Confirm pass; no implementation step needed**

Run: `cargo test -p shadowcat-server has_symbol_success_rule_feeds_net_successes --lib`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/dice/eval/success.rs
git commit -m "test(dice/m11b-3): pin HasSymbol success rule through the full evaluate path"
```

---

## Task 9: `CritTrigger` enum + symbol crits — **buddy-check this task before merge**

**Files:**
- Modify: `src/server/src/dice/spec.rs` (`CritSuccess`/`CritFail`: `threshold: i32` → `trigger: CritTrigger`)
- Modify: `src/server/src/dice/eval/crit.rs` (`score_die`'s signature and `reaches`)
- Modify: `src/server/src/dice/eval/success.rs:30` (the `crit::score_die(...)` call site — now needs the die's symbols)
- Modify: `src/server/src/dice/eval/expertise.rs:42` (`die_values`'s `crit::score_die(...)` call site)
- Modify (mechanical): every `CritSuccess { threshold: X, .. }`/`CritFail { threshold: X, .. }` literal (list below) — becomes `trigger: CritTrigger::AtLeast(X)`
- Test: `src/server/src/dice/eval/crit.rs`, `src/server/src/dice/eval/success.rs`

**Interfaces:**
- Consumes: `SuccessRule` enum idiom (Task 7, same match-on-enum pattern), `DieRecord.symbols` (Task 5).
- Produces: `CritTrigger::AtLeast(i32) | HasSymbol(Symbol)` — final piece of the symbolic-dice surface; nothing later in this plan consumes it directly (Task 10's expertise guard is orthogonal).

**⚠️ This task reopens the sealed, buddy-check-tier crit path (`eval::crit::score_die`, sealed since M11b-1). Per the design's §6/§12, do not merge this task's diff without a buddy-check pass.**

- [ ] **Step 1: Write the failing unit tests in `crit.rs`**

Add to `src/server/src/dice/eval/crit.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn symbol_crit_success_fires_on_matching_symbol() {
    let c = SuccessConfig {
        success: SuccessRule::Numeric { comp: Comparator::Gte, target: 100 }, // unreachable numeric target
        required_successes: None,
        tiers: vec![],
        crit_success: Some(CritSuccess {
            trigger: CritTrigger::HasSymbol("triumph".to_string()),
            extra_successes: 1,
            positive_counter: 1,
        }),
        crit_fail: None,
        expertise: 0,
    };
    let hit = score_die(Direction::HighWins, 0, &["triumph".to_string()], &c);
    assert!(hit.is_success && hit.extra_successes == 1 && hit.positive_counter == 1);
    let miss = score_die(Direction::HighWins, 0, &["blank".to_string()], &c);
    assert!(!miss.is_success);
}

#[test]
fn symbol_crit_trigger_is_direction_insensitive() {
    // A symbol is present or absent regardless of HighWins/LowWins — unlike
    // AtLeast, HasSymbol never flips.
    let c = SuccessConfig {
        success: SuccessRule::Numeric { comp: Comparator::Lte, target: 1 },
        required_successes: None,
        tiers: vec![],
        crit_success: Some(CritSuccess {
            trigger: CritTrigger::HasSymbol("despair".to_string()),
            extra_successes: 0,
            positive_counter: 1,
        }),
        crit_fail: None,
        expertise: 0,
    };
    let symbols = vec!["despair".to_string()];
    assert!(score_die(Direction::HighWins, 5, &symbols, &c).is_success);
    assert!(score_die(Direction::LowWins, 5, &symbols, &c).is_success);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat-server symbol_crit_success_fires symbol_crit_trigger_is_direction_insensitive --lib`
Expected: compile error — `CritTrigger`, the new `score_die` signature (`&[Symbol]` param), and `CritSuccess.trigger` don't exist yet.

- [ ] **Step 3: Add `CritTrigger` and convert `CritSuccess`/`CritFail`**

In `src/server/src/dice/spec.rs`, add above `CritSuccess`:

```rust
/// What makes a die's crit event fire. `AtLeast` is direction-aware (flips
/// under `LowWins`, exactly as the old bare `threshold: i32` did). `HasSymbol`
/// is direction-INSENSITIVE — a symbol is present or absent, there is no
/// "better end" to flip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CritTrigger {
    AtLeast(i32),
    HasSymbol(Symbol),
}
```

Change `CritSuccess`/`CritFail`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritSuccess {
    pub trigger: CritTrigger,
    pub extra_successes: i32,
    pub positive_counter: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritFail {
    pub trigger: CritTrigger,
    pub lost: i32,
    pub negative_counter: i32,
    pub allow_negative: bool,
}
```

- [ ] **Step 4: Update `score_die`'s signature and `reaches`**

In `src/server/src/dice/eval/crit.rs`, change:

```rust
use crate::dice::spec::{Direction, SuccessConfig};
```
to:
```rust
use crate::dice::spec::{CritTrigger, Direction, SuccessConfig, Symbol};
```

Change `reaches` and `score_die` from:

```rust
fn reaches(direction: Direction, value: i32, threshold: i32, is_success_event: bool) -> bool {
    match (direction, is_success_event) {
        (Direction::HighWins, true) | (Direction::LowWins, false) => value >= threshold,
        (Direction::HighWins, false) | (Direction::LowWins, true) => value <= threshold,
    }
}

pub fn score_die(direction: Direction, value: i32, cfg: &SuccessConfig) -> DieCrit {
    let mut out = DieCrit::default();
    if let Some(cs) = &cfg.crit_success {
        if reaches(direction, value, cs.threshold, true) {
            out.is_success = true;
            out.extra_successes = cs.extra_successes;
            out.positive_counter = cs.positive_counter;
        }
    }
    if let Some(cf) = &cfg.crit_fail {
        if reaches(direction, value, cf.threshold, false) {
            out.is_fail = true;
            out.lost = cf.lost;
            out.negative_counter = cf.negative_counter;
        }
    }
    out
}
```

to:

```rust
fn reaches(direction: Direction, value: i32, symbols: &[Symbol], trigger: &CritTrigger, is_success_event: bool) -> bool {
    match trigger {
        CritTrigger::AtLeast(threshold) => match (direction, is_success_event) {
            (Direction::HighWins, true) | (Direction::LowWins, false) => value >= *threshold,
            (Direction::HighWins, false) | (Direction::LowWins, true) => value <= *threshold,
        },
        // Direction-insensitive: presence/absence has no "better end" to flip.
        CritTrigger::HasSymbol(s) => symbols.contains(s),
    }
}

pub fn score_die(direction: Direction, value: i32, symbols: &[Symbol], cfg: &SuccessConfig) -> DieCrit {
    let mut out = DieCrit::default();
    if let Some(cs) = &cfg.crit_success {
        if reaches(direction, value, symbols, &cs.trigger, true) {
            out.is_success = true;
            out.extra_successes = cs.extra_successes;
            out.positive_counter = cs.positive_counter;
        }
    }
    if let Some(cf) = &cfg.crit_fail {
        if reaches(direction, value, symbols, &cf.trigger, false) {
            out.is_fail = true;
            out.lost = cf.lost;
            out.negative_counter = cf.negative_counter;
        }
    }
    out
}
```

- [ ] **Step 5: Fix `score_die`'s two production callers**

In `src/server/src/dice/eval/success.rs`, change:

```rust
        let dc = crit::score_die(spec.direction, r.value, cfg);
```
to:
```rust
        let dc = crit::score_die(spec.direction, r.value, &r.symbols, cfg);
```

In `src/server/src/dice/eval/expertise.rs`'s `die_values`, change:

```rust
            let dc = crit::score_die(direction, f, cfg);
```
to:
```rust
            // Expertise only ever adjusts a Numeric die's face (Task 10); a
            // Numeric die never carries symbols, so an empty slice is exact,
            // not a placeholder — re-verify once Task 10 lands.
            let dc = crit::score_die(direction, f, &[], cfg);
```

Also fix `expertise.rs`'s `score_pool` test helper (same call shape, same `&[]` reasoning) and `success.rs`'s test-module `cfg_of`-adjacent direct `score_die` calls if any exist beyond the two production sites above (check via `grep -n "score_die(" src/server/src/dice/eval/*.rs` — every remaining call site needs the new `symbols` argument; for test helpers scoring hand-built `DieRecord`s, pass `&r.symbols` where a real record is in scope, or `&[]` where the test's dice are known-`Numeric`).

- [ ] **Step 6: Fix every remaining compile site (mechanical)**

Wrap each `CritSuccess { threshold: X, extra_successes: E, positive_counter: P }` as `CritSuccess { trigger: CritTrigger::AtLeast(X), extra_successes: E, positive_counter: P }` (same field renaming for `CritFail`'s `threshold` → `trigger: CritTrigger::AtLeast(..)`), at:

- `src/server/src/dice/spec.rs:237` (the `success_config_serde_round_trips` test)
- `src/server/src/dice/proptests.rs:156,162,185,192`
- `src/server/src/dice/eval/crit.rs:71,87,102,119,124` (existing numeric-threshold tests — preserve their exact numeric behavior, only the field name/wrapping changes)
- `src/server/src/dice/eval/expertise.rs:238,379,384,503,512,570`
- `src/server/src/dice/eval/success.rs:130,174,272,277,368,481,507`

Run `cargo build --tests -p shadowcat-server` and fix any remaining "no field `threshold`"/"missing field `trigger`" errors at their reported locations until the build is clean.

- [ ] **Step 7: Run the symbol crit tests to verify they pass**

Run: `cargo test -p shadowcat-server symbol_crit_success_fires symbol_crit_trigger_is_direction_insensitive --lib`
Expected: PASS.

- [ ] **Step 8: Run the FULL crit + success + expertise test suites (regression-critical)**

Run: `cargo test -p shadowcat-server dice::eval::crit dice::eval::success dice::eval::expertise --lib`
Expected: all PASS, INCLUDING the M11b-2 differential-oracle test (`dp_matches_brute_force_oracle_over_a_random_corpus`) — this is the highest-value regression check in this task, since `expertise.rs`'s `die_values` now calls `score_die` with a threaded (empty) symbols slice, and any accidental behavior change there would silently break the oracle-pinned allocator.

- [ ] **Step 9: Write an end-to-end Triumph/Despair test**

Add to `src/server/src/dice/eval/success.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn triumph_symbol_crit_adds_success_and_positive_counter_end_to_end() {
    use crate::dice::spec::{CritSuccess, CritTrigger, Face};
    let faces = vec![
        Face { value: Some(0), symbols: vec!["blank".into()] },
        Face { value: Some(1), symbols: vec!["triumph".into()] },
    ];
    let cfg = SuccessConfig {
        success: SuccessRule::HasSymbol("triumph".to_string()),
        required_successes: None,
        tiers: vec![],
        crit_success: Some(CritSuccess {
            trigger: CritTrigger::HasSymbol("triumph".to_string()),
            extra_successes: 1,
            positive_counter: 1,
        }),
        crit_fail: None,
        expertise: 0,
    };
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup {
            count: 1,
            kind: DieKind::Faces { faces: faces.clone() },
            modifiers: vec![],
            label: None,
        }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(cfg),
    };
    let mut raws = RawRoll::default();
    raws.push(DieKind::Faces { faces: faces.clone() }, 1); // draws the "triumph" face
    let recs = crate::dice::eval::groups::resolve_group(
        match &spec.expr { Expr::Dice(g) => g, _ => unreachable!() },
        0,
        &raws.dice.clone(),
        &mut NoiseRng::from_seed(1),
        &mut raws,
    );
    raws.records = recs;
    raws.group_spans = vec![(0, 1)];
    let out = evaluate(&spec, &raws);
    // base success (HasSymbol) = 1, crit extra = 1 -> net 2.
    assert_eq!(out.successes, Some(2));
    assert_eq!(out.positive_counter, 1);
    assert_eq!(out.crit_successes, 1);
}
```

- [ ] **Step 10: Run test to verify it passes**

Run: `cargo test -p shadowcat-server triumph_symbol_crit_adds_success_and_positive_counter --lib`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/server/src/dice/spec.rs src/server/src/dice/eval/crit.rs src/server/src/dice/eval/success.rs src/server/src/dice/eval/expertise.rs src/server/src/dice/proptests.rs
git commit -m "feat(dice/m11b-3): CritTrigger enum (AtLeast/HasSymbol), symbol crits end-to-end"
```

- [ ] **Step 12: Flag for buddy-check**

Do not proceed to merge this branch until a buddy-check has run over this task's diff (`git diff` against the commit from Task 8), per the design's §6/§12 standing directive on the crit path. Record the outcome in this plan's "Buddy-check directives" section (below) before merge.

---

## Task 10: `symbol_counts` on `RollOutcome`

**Files:**
- Modify: `src/server/src/dice/outcome.rs` (`RollOutcome` gains `symbol_counts`)
- Modify: `src/server/src/dice/eval/success.rs` (compute it), `src/server/src/dice/eval/sum.rs` (report empty for Total mode)
- Test: `src/server/src/dice/eval/success.rs`

**Interfaces:**
- Consumes: `DieRecord.symbols` (Task 5).
- Produces: `RollOutcome.symbol_counts: BTreeMap<Symbol, i32>` — terminal output field, no later task reads it.

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/eval/success.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn symbol_counts_tallies_kept_dice_unconditionally() {
    use crate::dice::spec::Face;
    // Numeric success rule (not HasSymbol) — symbol_counts must STILL populate,
    // independent of which SuccessRule variant is active.
    let faces = vec![
        Face { value: Some(0), symbols: vec!["advantage".into()] },
        Face { value: Some(1), symbols: vec!["triumph".into(), "advantage".into()] },
    ];
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup {
            count: 2,
            kind: DieKind::Faces { faces: faces.clone() },
            modifiers: vec![],
            label: None,
        }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule::Numeric { comp: Comparator::Gte, target: 100 }, // never fires
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
            expertise: 0,
        }),
    };
    let mut raws = RawRoll::default();
    raws.push(DieKind::Faces { faces: faces.clone() }, 0);
    raws.push(DieKind::Faces { faces: faces.clone() }, 1);
    let recs = crate::dice::eval::groups::resolve_group(
        match &spec.expr { Expr::Dice(g) => g, _ => unreachable!() },
        0,
        &raws.dice.clone(),
        &mut NoiseRng::from_seed(1),
        &mut raws,
    );
    raws.records = recs;
    raws.group_spans = vec![(0, 2)];
    let out = evaluate(&spec, &raws);
    assert_eq!(out.symbol_counts.get("advantage"), Some(&2));
    assert_eq!(out.symbol_counts.get("triumph"), Some(&1));
    assert_eq!(out.successes, Some(0), "numeric rule never fires — sanity check");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server symbol_counts_tallies_kept_dice --lib`
Expected: compile error — `RollOutcome.symbol_counts` doesn't exist.

- [ ] **Step 3: Add `symbol_counts` to `RollOutcome`**

In `src/server/src/dice/outcome.rs`, add `use std::collections::BTreeMap;` at the top, and add to `RollOutcome` (after `negative_counter: i32,`):

```rust
    /// Per-symbol tallies over KEPT dice, computed unconditionally (independent
    /// of `SuccessRule`'s variant). Deterministic iteration order (`BTreeMap`).
    #[serde(default)]
    pub symbol_counts: BTreeMap<Symbol, i32>,
```

Add `Symbol` to `outcome.rs`'s spec import.

- [ ] **Step 4: Compute it in `evaluate_success`**

In `src/server/src/dice/eval/success.rs`, add `use std::collections::BTreeMap;` and, inside `evaluate_success`, after the existing per-die loop (which currently ends with `neg += dc.negative_counter;`), add symbol tallying to the SAME loop body:

Change:

```rust
    let mut base = 0i32;
    let (mut extra, mut lost) = (0i32, 0i32);
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    for r in records.iter_mut().filter(|r| r.kept) {
```

to:

```rust
    let mut base = 0i32;
    let (mut extra, mut lost) = (0i32, 0i32);
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    let mut symbol_counts: BTreeMap<crate::dice::spec::Symbol, i32> = BTreeMap::new();
    for r in records.iter_mut().filter(|r| r.kept) {
        for s in &r.symbols {
            *symbol_counts.entry(s.clone()).or_insert(0) += 1;
        }
```

(inserting the new block right after the `for` line, before the existing `if cfg.success...` body — order within the loop body doesn't matter since these are independent accumulators). Then add `symbol_counts,` to the function's final `RollOutcome { .. }` literal.

- [ ] **Step 5: Report empty `symbol_counts` in Total mode**

In `src/server/src/dice/eval/sum.rs`, add `symbol_counts: Default::default(),` to `evaluate_total`'s `RollOutcome { .. }` literal (Total mode has no symbol concept — always empty, matching the "0 in Total mode" convention already used for `crit_successes`/`crit_fails`/counters).

- [ ] **Step 6: Fix remaining compile sites (mechanical)**

Every other `RollOutcome { .. }` literal in the crate (test helpers in `outcome.rs`'s own tests, if any construct it directly — check via `grep -n "RollOutcome {" src/server/src/dice/**/*.rs`) needs `symbol_counts: Default::default(),` added.

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test -p shadowcat-server symbol_counts_tallies_kept_dice --lib`
Expected: PASS.

- [ ] **Step 8: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add src/server/src/dice/outcome.rs src/server/src/dice/eval/success.rs src/server/src/dice/eval/sum.rs
git commit -m "feat(dice/m11b-3): compute symbol_counts unconditionally over kept dice"
```

---

## Task 11: Expertise stays `Numeric`-only

**Files:**
- Modify: `src/server/src/dice/eval/expertise.rs:115-173` (`allocate`'s contributing-die filter)
- Test: `src/server/src/dice/eval/expertise.rs`

**Interfaces:**
- Consumes: `DieKind::Faces`/`is_ordered` (Tasks 4/6) — used only to construct the negative test case; `allocate`'s existing `bounds` map (Task 4's `unreachable!()` placeholder at `expertise.rs:130` gets its real fix here).
- Produces: nothing new consumed later — this is the design's §8/§F guard, final shape.

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/eval/expertise.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn expertise_never_allocates_to_an_ordered_faces_die() {
    use crate::dice::outcome::RawDie;
    use crate::dice::spec::Face;
    // One ordered Faces die (ranked, all-valued) + one Numeric die, both at a
    // value that COULD reach the target with 1 expertise point. Budget 1 point
    // total: it must land on the Numeric die, never the Faces die.
    let faces = vec![
        Face { value: Some(4), symbols: vec![] },
        Face { value: Some(5), symbols: vec![] },
    ];
    let c = SuccessConfig {
        success: SuccessRule::Numeric { comp: Comparator::Gte, target: 5 },
        required_successes: None,
        tiers: vec![],
        crit_success: None,
        crit_fail: None,
        expertise: 1,
    };
    let mut raws = RawRoll::default();
    let faces_id = raws.push(DieKind::Faces { faces: faces.clone() }, 0); // value 4, one step from 5
    let numeric_id = raws.push(DieKind::Numeric { min: 1, max: 6 }, 4); // value 4, one step from 5
    let mut records = vec![
        DieRecord { id: faces_id, group_index: 0, natural: 0, value: 4, kept: true, exploded: false, rerolled_from: None, crit_success: false, crit_fail: false, expertise: 0, label: None, symbols: vec![] },
        DieRecord { id: numeric_id, group_index: 0, natural: 4, value: 4, kept: true, exploded: false, rerolled_from: None, crit_success: false, crit_fail: false, expertise: 0, label: None, symbols: vec![] },
    ];
    allocate(Direction::HighWins, &c, &raws, &mut records);
    assert_eq!(records[0].expertise, 0, "ordered Faces die must never receive expertise points");
    assert_eq!(records[1].expertise, 1, "the Numeric die gets the point instead");
    assert_eq!(records[1].value, 5);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server expertise_never_allocates_to_an_ordered_faces_die --lib`
Expected: panics with an out-of-bounds or a `DieKind::Faces` variant hitting the `unreachable!()` placeholder from Task 4's `expertise.rs:130` fix (`bounds` map's closure), since `allocate` currently treats every kept record as contributing regardless of `DieKind`.

- [ ] **Step 3: Restrict the contributing-die filter to `Numeric`**

In `src/server/src/dice/eval/expertise.rs`'s `allocate`, change the `bounds` map (replacing Task 4's `unreachable!()` placeholder) and the `kept` filter:

```rust
    // Bounds per die: only Numeric dice have a defined [min,max] adjust range.
    let bounds: HashMap<DieId, (i32, i32)> = raws
        .dice
        .iter()
        .filter_map(|d| match d.kind {
            DieKind::Numeric { min, max } => Some((d.id, (min, max))),
            DieKind::Faces { .. } => None,
        })
        .collect();
    // Contributing dice = the pooled kept NUMERIC dice, in record order.
    // A Faces die (ordered or not) is excluded: `adjust`'s "+1 toward better
    // within [min,max]" has no defined meaning over an arbitrary face-list —
    // there is no contiguous numeric range to move within, and mutating
    // `value` to a non-face integer would desync the die's `symbols` (M11b-3
    // §8/§F, tighter than the design's literal "exclude only value:None faces").
    let kept: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kept && bounds.contains_key(&r.id))
        .map(|(i, _)| i)
        .collect();
```

(This replaces the old unconditional `bounds` map — which used the now-invalid `let DieKind::Numeric { min, max } = d.kind;` irrefutable pattern Task 4 already converted to a `match`-with-`unreachable!()` — and the old unconditional `kept` filter.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server expertise_never_allocates_to_an_ordered_faces_die --lib`
Expected: PASS.

- [ ] **Step 5: Run the FULL expertise test suite (oracle regression-critical)**

Run: `cargo test -p shadowcat-server dice::eval::expertise --lib`
Expected: all PASS, including `dp_matches_brute_force_oracle_over_a_random_corpus` — the oracle's corpus is entirely `Numeric` dice (via `raws_of`), so this filter change must be a no-op for every existing oracle case (the `bounds.contains_key` check is trivially true for every die the oracle ever constructs).

- [ ] **Step 6: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/dice/eval/expertise.rs
git commit -m "feat(dice/m11b-3): restrict expertise allocation to Numeric dice"
```

---

## Task 12: `recalculate` support for `Faces` base naturals (indices)

**Files:**
- Modify: `src/server/src/dice/recalc.rs:46-71` (the `RecalcOp::RerollDice` per-die loop — replace Task 4's `unreachable!()` placeholder)
- Test: `src/server/src/dice/recalc.rs`

**Interfaces:**
- Consumes: `DieKind::Faces` (Task 4), `face_value_and_symbols` visibility (Task 5's helper — currently private to `groups.rs`; this task needs it or an equivalent, see Step 3).

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/recalc.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn reroll_dice_redraws_a_fresh_index_for_a_faces_die() {
    use crate::dice::spec::Face;
    let faces = vec![
        Face { value: Some(1), symbols: vec![] },
        Face { value: Some(6), symbols: vec![] },
    ];
    let group = DiceGroup {
        count: 1,
        kind: DieKind::Faces { faces: faces.clone() },
        modifiers: vec![],
        label: None,
    };
    let spec = RollSpec {
        expr: Expr::Dice(group.clone()),
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig { difficulty: None, tiers: vec![] }),
    };
    let naturals = vec![RawDie { id: 0, kind: DieKind::Faces { faces: faces.clone() }, natural: 0 }];
    let raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 1, group_spans: vec![(0, 1)] };
    // ScriptedRng forces roll_uniform(rng, 0, 1) to draw index 1 (face value 6).
    let mut rng = ScriptedRng::new(vec![1]);
    let (raws2, out2) = recalculate(&spec, &raws, &[RecalcOp::RerollDice(vec![0])], &mut rng);
    assert_eq!(raws2.dice[0].natural, 1, "reroll drew face index 1");
    assert_eq!(out2.total, 6, "re-derived value reflects the new face's numeric value");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server reroll_dice_redraws_a_fresh_index_for_a_faces_die --lib`
Expected: panics on the Task-4 `unreachable!()` placeholder inside `RecalcOp::RerollDice`'s loop (`recalc.rs:52`).

- [ ] **Step 3: Replace the placeholder with a real `Faces`-aware redraw**

In `src/server/src/dice/recalc.rs`, change:

```rust
            RecalcOp::RerollDice(ids) => {
                for g in groups.iter_mut() {
                    for d in g.iter_mut() {
                        if ids.contains(&d.id) {
                            let (min, max) = match d.kind {
                                DieKind::Numeric { min, max } => (min, max),
                                DieKind::Faces { .. } => unreachable!("Faces recalc not yet wired (M11b-3 Task 11)"),
                            };
                            d.natural = roll_uniform(rng, min, max);
                        }
                    }
                }
            }
```

to:

```rust
            RecalcOp::RerollDice(ids) => {
                for g in groups.iter_mut() {
                    for d in g.iter_mut() {
                        if ids.contains(&d.id) {
                            d.natural = match &d.kind {
                                DieKind::Numeric { min, max } => roll_uniform(rng, *min, *max),
                                DieKind::Faces { faces } => roll_uniform(rng, 0, faces.len() as i32 - 1),
                            };
                        }
                    }
                }
            }
```

`RecalcOp::ReplaceDie`/`RecalcOp::RemoveDice` need no change: `ReplaceDie{id, natural}` already stores whatever `i32` the caller supplies as the new base natural (for a `Faces` die, the caller is responsible for supplying a valid index — same trust boundary as today's `Numeric` path, where the caller supplies a valid face value), and `RemoveDice` only filters by id, agnostic to `DieKind`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server reroll_dice_redraws_a_fresh_index_for_a_faces_die --lib`
Expected: PASS.

- [ ] **Step 5: Run the full dice test suite**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/dice/recalc.rs
git commit -m "feat(dice/m11b-3): recalculate redraws a fresh face index for Faces dice"
```

---

## Task 13: Property test extension + final integration pass

**Files:**
- Modify: `src/server/src/dice/proptests.rs` (extend an existing property, or add one, covering labeled + symbolic dice through the full `roll`/`evaluate`/`recalculate` path)
- Test: `src/server/src/dice/proptests.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-12.
- Produces: nothing further — this is the plan's closing verification task.

- [ ] **Step 1: Write a new property test**

Add to `src/server/src/dice/proptests.rs`:

```rust
proptest! {
    #[test]
    fn labeled_symbolic_pool_evaluate_is_deterministic(seed in any::<u64>(), count in 1u32..8) {
        // A labeled, symbolic (unordered) pool: evaluate must still be a pure
        // function of (spec, raws), exactly like the existing
        // `evaluate_is_deterministic` property for Numeric pools.
        let faces = vec![
            Face { value: None, symbols: vec!["success".into()] },
            Face { value: None, symbols: vec!["blank".into()] },
        ];
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count,
                kind: DieKind::Faces { faces },
                modifiers: vec![],
                label: Some("Pool".to_string()),
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::HasSymbol("success".to_string()),
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
                expertise: 0,
            }),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
        prop_assert_eq!(evaluate(&spec, &raws), evaluate(&spec, &raws));
        // Unordered symbolic pool: total must always be 0 (Task 6's is_ordered gate).
        prop_assert_eq!(evaluate(&spec, &raws).total, 0);
    }
}
```

Add `Face` to `proptests.rs`'s `use crate::dice::spec::{..}` import list.

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `cargo test -p shadowcat-server labeled_symbolic_pool_evaluate_is_deterministic --lib`
Expected: PASS if Tasks 1-12 are correctly implemented (this is an integration checkpoint, not new production code — same posture as Task 8). A failure here means re-examining the specific task whose guarantee this property exercises (determinism → Task 5's index draw; `total == 0` → Task 6's `is_ordered` gate).

- [ ] **Step 3: Run the ENTIRE dice module test suite one final time**

Run: `cargo test -p shadowcat-server dice:: --lib`
Expected: all PASS — every unit test, every property test, across all 13 tasks.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/dice/proptests.rs
git commit -m "test(dice/m11b-3): property-test labeled + symbolic pools through evaluate"
```

---

## Task 14: Codebase-skill update + reviewed gate

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-dice/SKILL.md` (or equivalent path — confirm exact filename via `Glob` before editing)

**Interfaces:** None — documentation task.

- [ ] **Step 1: Update the skill**

Update `shadowcat-codebase-dice`'s body to reflect: `DieKind::Faces`/`Face`/`Symbol`; the `is_ordered` gate and which pipeline stages it protects; `SuccessRule`/`CritTrigger` enum shapes (with the `Numeric`-default note); `DieRecord.label`/`symbols`; `RollOutcome.symbol_counts`/`by_label`/`compare_labels`; the expertise-stays-`Numeric`-only guard (§F); the `[label]` notation charset/duplicate rule; and mark M11b-3 (and thus M11b as a whole) DONE, removing "Still deferred" language from the skill's opening paragraph.

- [ ] **Step 2: Dispatch `shadowcat-spec-reviewer` on the skill diff**

Per the project's reviewed skill-update gate (CLAUDE.md), dispatch `shadowcat-spec-reviewer` to confirm the skill diff accurately captures every change in Tasks 1-13 — no omission, drift, or broken pointer. Do not merge until this review passes.

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/shadowcat-codebase-dice/
git commit -m "docs(dice/m11b-3): update codebase skill for labeled + custom-face dice"
```

---

## Buddy-check directives

Task 9 (`CritTrigger` enum + symbol crits) reopens the sealed, buddy-check-tier crit-scoring path (`eval::crit::score_die`) — per the design's §6/§12 standing directive, this task's diff must go through a buddy-check before the branch merges. Run it after Task 9's commit, before proceeding to Task 10 (or at the latest, before the final Task 14 merge) — either point is acceptable as long as the check happens before merge. Record the buddy-check's outcome (pass / issues found + fixed) here once run.

## Model/Effort directives

This plan was written mainline (in the coordinating session, Sonnet 5, default effort) rather than dispatched to a named `sdd-plan-writer-*` agent — the user explicitly chose mainline continuation over dispatch when prompted at the tier-switch checkpoint, deviating from the project CLAUDE.md's default (which recommends `sdd-plan-writer-sonnet` for this design's mechanical-decomposition profile). Execution should still follow the project's standard per-task tiering: `shadowcat-coder` (sonnet/medium) for implementation, escalating to `shadowcat-coder-opus` on a reported block; `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (sonnet/high, escalating to their opus twins) at review checkpoints, per `~/.claude/docs/sdd-model-effort-tiers.md`.
