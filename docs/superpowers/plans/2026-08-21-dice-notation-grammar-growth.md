# Dice-Notation Grammar Growth — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. Invoke `shadowcat-codebase-dice` (and, for Task 4,
> `shadowcat-codebase-formula`) before starting any task.

**Goal:** Add two independent, additive grammar extensions to `dice::notation`: (1) six math
functions (`floor`/`ceil`/`round`/`abs`/`min`/`max`) as a new `Expr::Call` AST production, and
(2) notation syntax for the already-fully-built `Tier`/`CritSuccess`/`CritFail` data model
(`tr<offset>[:<value>][<label>]`, `xs<N>[:<extra>[:<counter>]]`, `xf<N>[:<lost>[:<counter>]][!]`).
Closes `docs/TODO.md` bucket-C sub-project 4.

**Spec:** `docs/superpowers/specs/2026-08-21-dice-notation-grammar-growth-design.md` — read in
full before implementing any task. Every design fork it resolves (the v1 math-function set, the
`Expr::Call` AST shape, group-index-cursor threading through `Call` args, `tr`/`xs`/`xf` syntax,
AtLeast-only crit triggers for v1, and the `NOTATION_KEYWORDS` parity scoping — math functions do
NOT join that parity set, only `tr`/`xs`/`xf` do) is FINAL and not open for re-litigation. This
plan resolves two points the spec left underspecified; both are called out explicitly at their
task (Task 3's `tr` grammar for negative offsets, Task 4's mode-gating for `xs`/`xf`) rather than
silently assumed.

## Global Constraints

- **Iron rule (campaign directive, binding on every subagent this campaign):** No deferrals of
  existing work or new work as it comes up — fix it now unless the user gives EXPRESS
  authorization. The only exception is a bug/TODO with a genuine blocker already logged in a
  milestone in `docs/PLAN.md` that has not started. When faced with a design fork, determine the
  best long-term shape in keeping with our plans and goals and implement accordingly; only ask the
  user if that question is genuinely unanswerable. Churn is not a concern. **This paragraph must
  be copied verbatim into every subagent dispatched for this campaign.**
- **No lint suppressions of any kind.** `#[allow(dead_code)]`, `#[allow(unused*)]`,
  `#[allow(clippy::*)]`, and `#[expect(...)]` are ALL forbidden — no exceptions, no per-instance
  sign-off requests in this campaign. Fix the code, make it live, `#[cfg(test)]`-scope test-only
  items, or delete them. Finding one already in the tree during this work is a defect to fix, not
  a precedent to follow.
- **RULE 15:** cite symbols (type/function/method names) in code comments, never file names or
  line numbers.
- **No ephemeral referents in CODE comments:** no milestone ids, no dated doc pointers, no
  history/process narration in `.rs`/`.ts` source comments. This plan and the design spec are
  `docs/superpowers/` artifacts and are exempt from this rule themselves, but nothing from either
  may be copy-pasted into a CODE comment as a citation (e.g. never write `// per the grammar-growth
  spec` in a `.rs` file — state the invariant/reason directly instead).
- **Every new/changed item needs a doc comment.** `src/server/src/dice/**` and
  `src/server/src/chat/rolls.rs` enforce `#![deny(missing_docs)]` +
  `#![deny(clippy::missing_docs_in_private_items)]` per-module.
- **Never fork a decision across two paths.** `Expr::Call`'s group-index-cursor threading in
  `eval::sum::fold` and `eval::eval::roll_expr` must mirror `Expr::Bin`'s existing two-child
  threading exactly (a for-loop over `args`, generalizing the pattern already established) — do
  not invent a second, different recursion shape for the new variant.
- Never delete files with `rm`/`Remove-Item`; use `trash`.
- **Rust CI gate battery** (run from the repository root — the workspace `Cargo.toml` at
  `C:\Dev\Shadowcat\Cargo.toml` has members `src/server` and `src/server/test-support`), required
  after every task that touches `src/server/**` (Tasks 1–4), all must exit 0:
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo test --all`
  4. `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`
  5. `cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc`
- **Client gate battery**, required after Task 4 only (the only task touching
  `src/client/formula/**`), all must exit 0:
  6. `pnpm --filter @shadowcat/formula typecheck`
  7. `pnpm --filter @shadowcat/formula test`
  8. `pnpm run test:scripts` (runs `scripts/check-notation-modifier-parity.test.mjs`, which reads
     `NOTATION_KEYWORDS` from `template.ts` and the `match id.as_str()` block from
     `dice/notation/parser.rs` directly — proves parity mechanically rather than by inspection)
- **Reviewed Skill-Update Gate:** this work touches `src/server/src/dice/spec.rs`,
  `src/server/src/dice/notation/lexer.rs`, `src/server/src/dice/notation/parser.rs`,
  `src/server/src/dice/notation/mod.rs`, `src/server/src/dice/eval/mod.rs`,
  `src/server/src/dice/eval/sum.rs`, `src/server/src/dice/recalc.rs`,
  `src/server/src/chat/rolls.rs`, and `src/client/formula/src/template.ts` — squarely
  `shadowcat-codebase-dice` and `shadowcat-codebase-formula` territory, with one small
  cross-reference in `shadowcat-codebase-chat`. Task 5 updates all three skills, dispatches
  `shadowcat-spec-reviewer` on the skill diffs, and bumps
  `.claude/.claude-plugin/plugin.json`'s `version` from `1.0.59` to `1.0.60`.
- **TODO.md cleanup:** Task 5 removes the "math fns (floor/ceil/round/abs/min/max) + crit-event /
  tier-ladder notation syntax" line from `docs/TODO.md` bucket-C sub-project 4 once Task 4 is
  verified end-to-end (the design is now implemented, not merely scheduled).

## Task Decomposition Rationale

Five tasks. `Expr::Call`'s AST addition and its evaluator-side group-index-cursor threading
(Task 1) are inseparable for compilation — Rust's match exhaustiveness forces every existing
`match expr { Expr::Dice(..) | Expr::Const(..) | Expr::Neg(..) | Expr::Bin{..} }` site
(`eval::eval::roll_expr`, `eval::sum::fold`, `eval::sum::collect_labeled_consts`,
`dice::recalc::rederive`, `chat::rolls::walk_groups`) to gain a `Call` arm the moment the variant
exists, so there is no way to land the AST addition without its evaluator wiring in the same
compiling unit. **This is a deliberate departure from the dispatching brief's suggested
(1) AST-only / (2) evaluator-only split** — that split would require either a placeholder `Call`
arm (banned) or a non-compiling intermediate state. Task 1 therefore carries both, and is flagged
for the heavier two-reviewer tier the spec's §5 mandates for this exact reason (the
group-index-cursor extension is genuinely correctness-critical, mirroring the dice skill's
existing blanket review-tier mandate for `eval::sum`/`eval::classify`/`eval::success`-touching
changes). Task 2 is a dedicated, test-only follow-up: the full-stack (parse → roll → evaluate)
integration tests, including the spec's explicitly-called-out `floor(1d20/2)` group-boundary case,
kept as its own task/commit so that this specific evidence is independently reviewable. Task 2
carries the same heavier review-tier flag.

1. **`Expr::Call`/`FnName` AST + grammar + arity checking + all exhaustive-match plumbing +
   `apply_fn` + parser/unit-level tests.** (HEAVIER REVIEW TIER)
2. **Full-stack evaluator integration tests for math-function calls**, including the
   group-boundary-preservation case. (HEAVIER REVIEW TIER, same reason as Task 1)
3. **`tr<offset>[:<value>][<label>]` tier-ladder modifier** — parser scratch state, grammar, `Vec`
   threading into both `TotalConfig.tiers`/`SuccessConfig.tiers` (replacing the hardcoded
   `tiers: vec![]`), stale-comment cleanup in `chat::rolls`, and tests including the
   duplicate-offset rejection now reachable from notation.
4. **`xs<N>[:<extra>[:<counter>]]` / `xf<N>[:<lost>[:<counter>]][!]` crit-trigger modifiers** +
   `NOTATION_KEYWORDS` parity update, landed in the same commit, plus tests.
5. **Codebase-skill doc-sync pass** (dice/formula/chat skills), `shadowcat-spec-reviewer`
   dispatch, plugin version bump, `docs/TODO.md` cleanup.

---

## Task 1: `Expr::Call`/`FnName` AST, grammar, arity checking, evaluator plumbing (HEAVIER REVIEW TIER)

**Files:**
- Modify: `src/server/src/dice/spec.rs` (add `FnName`, add `Expr::Call`, tests)
- Modify: `src/server/src/dice/notation/lexer.rs` (add `Token::Comma`, lexer test)
- Modify: `src/server/src/dice/notation/parser.rs` (`factor` gains a `fn_call` branch, `fn_call`
  helper, tests)
- Modify: `src/server/src/dice/eval/mod.rs` (`roll_expr` gains a `Call` arm)
- Modify: `src/server/src/dice/eval/sum.rs` (`fold` and `collect_labeled_consts` gain `Call` arms,
  new `apply_fn`, tests)
- Modify: `src/server/src/dice/recalc.rs` (`rederive` gains a `Call` arm)
- Modify: `src/server/src/chat/rolls.rs` (`walk_groups` gains a `Call` arm — SECURITY-RELEVANT: without
  this, a dice group nested inside a `Call` argument bypasses `MAX_ROLL_DICE` entirely; see
  Task 1 Step 7 test)
- Modify: `src/server/src/dice/mod.rs` (re-export `FnName`)

### Step 1: `FnName` + `Expr::Call` in `spec.rs`

In `src/server/src/dice/spec.rs`, insert immediately after the `BinOp` enum (after line 208, before
the `ConstTerm` struct):

```rust
/// A math function name recognized by the `factor := ... | fn_call` grammar production.
/// Fixed arity per variant (`FnName::arity`): `Floor`/`Ceil`/`Round`/`Abs` take exactly 1
/// argument, `Min`/`Max` take exactly 2 — checked at parse time
/// (`dice::notation::parser::P::fn_call`), never at evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FnName {
    /// Round toward negative infinity.
    Floor,
    /// Round toward positive infinity.
    Ceil,
    /// Round to nearest, ties AWAY FROM ZERO (Rust's `f64::round` semantics) — differs from
    /// `@shadowcat/formula`'s own `Round`, which is JS-native and ties toward positive infinity.
    /// The two implementations are deliberately independent (see the module-level design
    /// rationale in the sub-project spec); this crate never calls into `@shadowcat/formula`.
    Round,
    /// Absolute value.
    Abs,
    /// The lesser of two arguments.
    Min,
    /// The greater of two arguments.
    Max,
}

impl FnName {
    /// The fixed argument count this function requires.
    /// `dice::notation::parser` checks a call's actual argument count against this at parse
    /// time; `Expr::Call` itself carries no arity guarantee once constructed.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::spec::FnName;
    /// assert_eq!(FnName::Floor.arity(), 1);
    /// assert_eq!(FnName::Min.arity(), 2);
    /// ```
    pub fn arity(self) -> usize {
        match self {
            FnName::Floor | FnName::Ceil | FnName::Round | FnName::Abs => 1,
            FnName::Min | FnName::Max => 2,
        }
    }
}
```

Then modify the `Expr` enum (around line 227) to add the `Call` variant after `Neg`:

```rust
/// Roll expression AST. Sum mode folds this to a total; SuccessCount mode ignores
/// the arithmetic and pools the dice reachable from `Dice` nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// A rolled dice group.
    Dice(DiceGroup),
    /// A constant term.
    Const(ConstTerm),
    /// A binary arithmetic node.
    Bin {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// Unary negation.
    Neg(Box<Expr>),
    /// A math function call (`floor`/`ceil`/`round`/`abs`/`min`/`max`), reached only via the
    /// `factor := ... | fn_call` grammar production. `eval::sum::fold` and
    /// `eval::eval::roll_expr` both recurse through `args` in left-to-right order, threading the
    /// SAME group-index cursor sequentially through each argument — the same mechanism
    /// `Bin{lhs, rhs}` already uses for two children, generalized to N. A `Call` node itself
    /// introduces no new dice groups and consumes no group-index slots beyond what its
    /// arguments consume.
    Call {
        /// Which function.
        name: FnName,
        /// Evaluated argument expressions, left-to-right; length is checked against
        /// `name.arity()` at parse time.
        args: Vec<Expr>,
    },
}
```

### Step 2: `spec.rs` tests

Add to `spec.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn fn_name_arity_matches_grammar() {
        assert_eq!(FnName::Floor.arity(), 1);
        assert_eq!(FnName::Ceil.arity(), 1);
        assert_eq!(FnName::Round.arity(), 1);
        assert_eq!(FnName::Abs.arity(), 1);
        assert_eq!(FnName::Min.arity(), 2);
        assert_eq!(FnName::Max.arity(), 2);
    }

    #[test]
    fn call_expr_serde_round_trips() {
        let spec = RollSpec {
            expr: Expr::Call {
                name: FnName::Min,
                args: vec![
                    Expr::Const(ConstTerm {
                        value: 3,
                        label: None,
                    }),
                    Expr::Const(ConstTerm {
                        value: 5,
                        label: None,
                    }),
                ],
            },
            direction: Direction::HighWins,
            mode: Mode::Total(TotalConfig {
                difficulty: None,
                tiers: vec![],
            }),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
    }
```

### Step 3: `Token::Comma` in `lexer.rs`

In `src/server/src/dice/notation/lexer.rs`, add a new `Token` variant after `Label`:

```rust
    /// A `[bracketed]` label's text (printable ASCII + spaces only).
    Label(String),
    /// `,` — separates arguments in a `fn_call`.
    Comma,
```

Add the matching `Display` arm inside `impl std::fmt::Display for Token`:

```rust
            Token::Label(s) => write!(f, "the label '{s}'"),
            Token::Comma => write!(f, "','"),
```

Add the lexing arm inside `lex`, alongside the other single-char punctuation (after the `')'` arm):

```rust
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
```

Add a lexer test:

```rust
    #[test]
    fn lex_comma_separates_call_arguments() {
        let toks = lex("min(3,5)").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Ident("min".into()),
                Token::LParen,
                Token::Int(3),
                Token::Comma,
                Token::Int(5),
                Token::RParen,
            ]
        );
    }
```

### Step 4: `factor()` gains the `fn_call` branch in `parser.rs`

In `src/server/src/dice/notation/parser.rs`, update the import line to add `FnName`:

```rust
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, DiceGroup, DieKind, Direction, ExplodeKind, Expr, FnName,
    GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, TotalConfig,
};
```

In `factor()`, insert a new arm immediately before the final catch-all `other => Err(...)` arm:

```rust
            Some(Token::Ident(name)) => {
                let name = name.clone();
                if matches!(self.toks.get(self.pos + 1), Some(Token::LParen)) {
                    self.fn_call(name)
                } else {
                    Err(ParseError::Unexpected(format!(
                        "expected a number or dice expression, found {}",
                        describe_token(self.peek())
                    )))
                }
            }
```

(This preserves the existing catch-all `other => Err(...)` arm unchanged, below the new arm — an
`Ident` not immediately followed by `(` falls into the new arm's own `else`, not the catch-all.)

Add a new method on `impl P`, placed after `factor`:

```rust
    /// Parses `fn_call := ident '(' expr (',' expr)* ')'`. Called from `factor` after peeking
    /// (not yet consuming) the leading `Ident` and confirming it is immediately followed by `(`
    /// — the only place an `Ident` is recognized as a function name rather than a dice-group
    /// modifier keyword. `name` is the already-lowercased identifier text (the lexer lowercases
    /// every `Ident` it emits). Checks the parsed argument count against `FnName::arity` before
    /// returning, so an `Expr::Call` this parser produces always carries the exact argument
    /// count its `name` requires.
    fn fn_call(&mut self, name: String) -> Result<Expr, ParseError> {
        let fn_name = match name.as_str() {
            "floor" => FnName::Floor,
            "ceil" => FnName::Ceil,
            "round" => FnName::Round,
            "abs" => FnName::Abs,
            "min" => FnName::Min,
            "max" => FnName::Max,
            other => return Err(ParseError::Unexpected(format!("unknown function '{other}'"))),
        };
        self.bump(); // the Ident
        match self.bump() {
            Some(Token::LParen) => {}
            other => {
                return Err(ParseError::Unexpected(format!(
                    "expected '(', found {}",
                    describe_token(other.as_ref())
                )))
            }
        }
        let mut args = vec![self.expr()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.bump();
            args.push(self.expr()?);
        }
        match self.bump() {
            Some(Token::RParen) => {}
            other => {
                return Err(ParseError::Unexpected(format!(
                    "expected ')', found {}",
                    describe_token(other.as_ref())
                )))
            }
        }
        let expected = fn_name.arity();
        if args.len() != expected {
            return Err(ParseError::Unexpected(format!(
                "function '{name}' expects {expected} argument(s), found {}",
                args.len()
            )));
        }
        Ok(Expr::Call {
            name: fn_name,
            args,
        })
    }
```

Also update `factor`'s own doc comment (`/// factor := '-' factor | '(' expr ')' | dice | int` ...) to:

```rust
    /// `factor := '-' factor | '(' expr ')' | fn_call | dice | int` — the leaf level;
    /// dice factors continue into `modifiers`; `fn_call` is `ident '(' expr (',' expr)* ')'`
    /// (see `fn_call`).
```

There is a SECOND, separate grammar-summary doc comment — on `pub fn parse` itself, above `factor`
— that also spells out this grammar and must be kept in sync or it goes stale. Find:

```rust
/// Recursive-descent parser: `expr := term (('+'|'-') term)*`;
/// `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | dice | int`;
/// a dice factor is `int 'd' int modifier*`. `ctx` supplies the ambient mode/
/// direction the notation string itself does not encode; an explicit `cs`/`cf`
/// forces `SuccessCount` regardless of `ctx.mode`.
pub fn parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError> {
```

Replace with:

```rust
/// Recursive-descent parser: `expr := term (('+'|'-') term)*`;
/// `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | fn_call | dice
/// | int`; a dice factor is `int 'd' int modifier*`; `fn_call` is `ident '(' expr (',' expr)*
/// ')'` (see `fn_call`). `ctx` supplies the ambient mode/
/// direction the notation string itself does not encode; an explicit `cs`/`cf`
/// forces `SuccessCount` regardless of `ctx.mode`.
pub fn parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError> {
```

### Step 5: parser tests in `parser.rs`

Add to `parser.rs`'s `#[cfg(test)] mod tests` (which already has `use crate::dice::spec::*;`):

```rust
    #[test]
    fn parses_min_call_with_two_bare_consts() {
        let spec = parse("min(3,5)", ParseContext::default()).unwrap();
        assert_eq!(
            spec.expr,
            Expr::Call {
                name: FnName::Min,
                args: vec![
                    Expr::Const(ConstTerm {
                        value: 3,
                        label: None
                    }),
                    Expr::Const(ConstTerm {
                        value: 5,
                        label: None
                    }),
                ],
            }
        );
    }

    #[test]
    fn parses_floor_call_wrapping_a_dice_group() {
        let spec = parse("floor(1d20/2)", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Call {
                name: FnName::Floor,
                args,
            } => {
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Expr::Bin { op: BinOp::Div, .. }));
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn parses_nested_min_max_calls() {
        let spec = parse("max(min(1,2),3)", ParseContext::default()).unwrap();
        assert!(matches!(
            spec.expr,
            Expr::Call {
                name: FnName::Max,
                ..
            }
        ));
    }

    #[test]
    fn rejects_min_with_wrong_arity() {
        match parse("min(3)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => {
                assert!(msg.contains("min"), "{msg}");
                assert!(msg.contains('2'), "{msg}");
            }
            other => panic!("expected an arity Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_floor_with_wrong_arity() {
        match parse("floor(3,4)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => {
                assert!(msg.contains("floor"), "{msg}");
                assert!(msg.contains('1'), "{msg}");
            }
            other => panic!("expected an arity Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_function_name() {
        match parse("foo(3)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => assert!(msg.contains("unknown function 'foo'")),
            other => panic!("expected an unknown-function Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn bare_ident_not_followed_by_lparen_is_not_a_function_call() {
        assert!(parse("floor", ParseContext::default()).is_err());
    }
```

Also extend `notation/mod.rs`'s `real_parse_failures_render_without_debug_artifacts` test's
`inputs` array (in `src/server/src/dice/notation/mod.rs`) with two new entries, to keep the
Display-no-debug-artifacts pin covering the two new error shapes:

```rust
        let inputs = [
            "4d6 @ 2",                  // lexer: unexpected character
            "2d6 2d6",                  // trailing input
            "4d",                       // expect_int: missing sides
            "(1d4+1",                   // expect ')'
            "4d6xyz",                   // unknown modifier
            "6d6r",                     // cmp_target_required
            "café",                     // non-ASCII
            "999999999999999999999999", // invalid number literal
            "min(3)",                   // fn_call: wrong arity
            "foo(3)",                   // fn_call: unknown function name
        ];
```

### Step 6: `roll_expr`'s `Call` arm in `eval/mod.rs`

In `src/server/src/dice/eval/mod.rs`, add a `Call` arm to `roll_expr` after the `Bin` arm:

```rust
fn roll_expr(expr: &Expr, rng: &mut dyn RngSource, raws: &mut RawRoll, group_index: &mut usize) {
    match expr {
        Expr::Dice(group) => {
            // ... unchanged ...
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => roll_expr(inner, rng, raws, group_index),
        Expr::Bin { lhs, rhs, .. } => {
            roll_expr(lhs, rng, raws, group_index);
            roll_expr(rhs, rng, raws, group_index);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                roll_expr(arg, rng, raws, group_index);
            }
        }
    }
}
```

Update the function's doc comment to add:

```rust
/// `group_index` increments once per `Dice` node in AST left-to-right order —
/// the same order `eval::sum::evaluate_total` walks, so a `DieRecord`'s stamped
/// `group_index` always matches the `Dice` node that produced it. A `Call` node's
/// arguments are walked in the same left-to-right order, threading the same cursor
/// through each — generalizing `Bin{lhs, rhs}`'s two-child threading to N children;
/// `Call` itself introduces no new dice groups.
```

### Step 7: `fold`, `apply_fn`, and `collect_labeled_consts` in `eval/sum.rs`

In `src/server/src/dice/eval/sum.rs`, update the import line:

```rust
use crate::dice::spec::{BinOp, ConstTerm, Expr, FnName, RollSpec, TotalConfig};
```

Add a `Call` arm to `collect_labeled_consts`, after the general `Bin` arm:

```rust
fn collect_labeled_consts(expr: &Expr, sign: i32, out: &mut Vec<ConstTerm>) {
    match expr {
        Expr::Const(c) => { /* unchanged */ }
        Expr::Dice(_) => {}
        Expr::Neg(inner) => collect_labeled_consts(inner, -sign, out),
        Expr::Bin {
            op: BinOp::Sub,
            lhs,
            rhs,
        } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, -sign, out);
        }
        Expr::Bin { lhs, rhs, .. } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, sign, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_labeled_consts(arg, sign, out);
            }
        }
    }
}
```

Add a `Call` arm to `fold`, after the `Bin` arm, and a new private `apply_fn`:

```rust
fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => c.value as i64,
        Expr::Neg(inner) => fold(inner, raws, next_group)
            .checked_neg()
            .unwrap_or(i64::MAX),
        Expr::Dice(_) => { /* unchanged */ }
        Expr::Bin { op, lhs, rhs } => { /* unchanged */ }
        Expr::Call { name, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for arg in args {
                vals.push(fold(arg, raws, next_group));
            }
            apply_fn(*name, &vals)
        }
    }
}

/// Applies a math function to its already-`fold`ed argument values. Indexes
/// defensively (`unwrap_or(0)` via the local `a` closure, mirroring `fold`'s own
/// Div-by-zero-to-0 convention) rather than panicking on an argument count that
/// disagrees with `name.arity()` — unreachable from `dice::notation::parser`-constructed
/// input (arity is checked at parse time there), but this crate's own types stay
/// unvalidated by design for a hand-constructed `RollSpec`.
/// `Floor`/`Ceil`/`Round` route through `f64` then cast back to `i64`. Over the
/// purely-integer arithmetic `fold` already produces for every other `Expr` variant
/// (including `BinOp::Div`, which truncates toward zero rather than producing a
/// fraction), this is a no-op today for any argument built from `+`/`-`/`*`/`/` — these
/// three exist for functional parity with `@shadowcat/formula`'s own function set, not
/// to change an already-integer input's value in this v1.
fn apply_fn(name: FnName, args: &[i64]) -> i64 {
    let a = |i: usize| args.get(i).copied().unwrap_or(0);
    match name {
        FnName::Floor => (a(0) as f64).floor() as i64,
        FnName::Ceil => (a(0) as f64).ceil() as i64,
        FnName::Round => (a(0) as f64).round() as i64,
        FnName::Abs => a(0).saturating_abs(),
        FnName::Min => a(0).min(a(1)),
        FnName::Max => a(0).max(a(1)),
    }
}
```

Update `fold`'s doc comment to add mention of `Call`:

```rust
/// Recursive Total-mode fold: consts as-is, dice groups as their kept-record
/// sums (consumed left-to-right via `next_group`), operators saturating, `Call`
/// nodes folding each argument in left-to-right order (the same cursor-threading
/// `Bin` uses, generalized to N children) then applying `apply_fn`.
```

### Step 8: `apply_fn` unit tests in `eval/sum.rs`

Add to `sum.rs`'s `#[cfg(test)] mod tests` (update its `use` line to add `FnName`):

```rust
    use crate::dice::spec::{
        BinOp, ConstTerm, DiceGroup, DieKind, Direction, Expr, FnName, Mode, RollSpec, Tier,
        TotalConfig,
    };
```

```rust
    #[test]
    fn apply_fn_floor_ceil_round_are_noops_over_integer_input() {
        assert_eq!(super::apply_fn(FnName::Floor, &[7]), 7);
        assert_eq!(super::apply_fn(FnName::Ceil, &[-3]), -3);
        assert_eq!(super::apply_fn(FnName::Round, &[4]), 4);
    }

    #[test]
    fn apply_fn_abs_min_max() {
        assert_eq!(super::apply_fn(FnName::Abs, &[-7]), 7);
        assert_eq!(super::apply_fn(FnName::Min, &[3, 5]), 3);
        assert_eq!(super::apply_fn(FnName::Max, &[3, 5]), 5);
    }

    #[test]
    fn apply_fn_defends_against_missing_args_instead_of_panicking() {
        // Unreachable from parser-constructed input (arity is checked at parse
        // time), but a hand-constructed `Expr::Call` with too few args must not
        // panic -- mirrors `fold`'s own Div-by-zero-to-0 convention.
        assert_eq!(super::apply_fn(FnName::Min, &[3]), 0); // missing arg defaults to 0
    }
```

### Step 9: `rederive`'s `Call` arm in `recalc.rs`

In `src/server/src/dice/recalc.rs`, add a `Call` arm to `rederive` after the `Bin` arm:

```rust
fn rederive(
    expr: &Expr,
    groups: &[Vec<RawDie>],
    group_index: &mut usize,
    rng: &mut dyn RngSource,
    out: &mut RawRoll,
) {
    match expr {
        Expr::Dice(group) => { /* unchanged */ }
        Expr::Const(_) => {}
        Expr::Neg(inner) => rederive(inner, groups, group_index, rng, out),
        Expr::Bin { lhs, rhs, .. } => {
            rederive(lhs, groups, group_index, rng, out);
            rederive(rhs, groups, group_index, rng, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                rederive(arg, groups, group_index, rng, out);
            }
        }
    }
}
```

### Step 10: `walk_groups`'s `Call` arm in `chat/rolls.rs` (SECURITY-RELEVANT)

In `src/server/src/chat/rolls.rs`, add a `Call` arm to `walk_groups` after the `Bin` arm:

```rust
fn walk_groups<'a>(expr: &'a Expr, total: &mut u64, groups: &mut Vec<&'a DiceGroup>) {
    match expr {
        Expr::Dice(group) => {
            *total += group.count as u64;
            groups.push(group);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => walk_groups(inner, total, groups),
        Expr::Bin { lhs, rhs, .. } => {
            walk_groups(lhs, total, groups);
            walk_groups(rhs, total, groups);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                walk_groups(arg, total, groups);
            }
        }
    }
}
```

Update the function's doc comment to add: "Recurses into `Expr::Call`'s `args` too, so a dice
group nested inside a math-function argument still counts toward `MAX_ROLL_DICE` and is validated
by the per-group cap checks below — a `Call` node is not a way to smuggle dice groups past this
walk."

Add a regression test to `chat/rolls.rs`'s `#[cfg(test)] mod tests` (near the other `MAX_ROLL_DICE`
tests):

```rust
    #[test]
    fn dice_nested_inside_a_call_argument_still_counts_toward_max_roll_dice() {
        // `walk_groups` must recurse into `Expr::Call`'s args -- otherwise a dice group
        // wrapped in floor/ceil/round/abs/min/max would bypass MAX_ROLL_DICE entirely.
        match validate_formula("floor(101d6/2)", total_ctx()) {
            Err(RollError::TooManyDice(101)) => {}
            other => panic!("expected TooManyDice(101), got {other:?}"),
        }
    }
```

### Step 11: re-export `FnName` from `dice::mod`

In `src/server/src/dice/mod.rs`, update the `pub use spec::{...}` list to insert `FnName`
alphabetically after `Expr`:

```rust
pub use spec::{
    BinOp, Comparator, CritFail, CritSuccess, DiceGroup, DieId, DieKind, Direction, ExplodeKind,
    Expr, FnName, GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, Tier, TotalConfig,
};
```

### Verification

Run the full Rust CI gate battery (Global Constraints items 1–5). This is the HEAVIER REVIEW TIER
task: dispatch both `shadowcat-spec-reviewer` and `shadowcat-code-reviewer` on this task's diff
before proceeding to Task 2, per the dice skill's blanket mandate for `eval::sum`-touching changes.

---

## Task 2: Full-stack evaluator integration tests for math-function calls (HEAVIER REVIEW TIER)

**Files:**
- Modify: `src/server/src/dice/eval/sum.rs` (test-only additions)

No production code changes — this task is entirely test coverage proving Task 1's group-index-cursor
threading is correct end-to-end (parse → roll → evaluate), which is the specific correctness-critical
risk the spec's §5 calls out.

### Step 1: group-boundary-preservation test (the spec's explicitly-named case)

Add to `sum.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn floor_call_wrapping_dice_group_preserves_group_boundary() {
        // The correctness-critical case per the sub-project spec's §5-and-§3.3: a Call node
        // must not disturb the group_index cursor `fold` threads through it -- the underlying
        // d20's individual result must still be recoverable in `raws.records`, not just folded
        // into the (here, numerically no-op) floored total.
        let spec = notation::parse("floor(1d20/2)", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(3));
        let out = evaluate(&spec, &raws);
        assert_eq!(
            raws.records.len(),
            1,
            "the 1d20 group still produces exactly one record"
        );
        let d20_value = raws.records[0].value as i64;
        assert_eq!(out.total, d20_value / 2);
    }
```

### Step 2: multi-argument group-index threading across two dice groups

```rust
    #[test]
    fn min_call_across_two_dice_groups_threads_group_index_through_both_args() {
        let spec = notation::parse("min(2d6, 1d20)", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(9));
        let out = evaluate(&spec, &raws);
        let g0: i64 = raws
            .records
            .iter()
            .filter(|r| r.group_index == 0 && r.kept)
            .map(|r| r.value as i64)
            .sum();
        let g1: i64 = raws
            .records
            .iter()
            .filter(|r| r.group_index == 1 && r.kept)
            .map(|r| r.value as i64)
            .sum();
        assert_eq!(out.total, g0.min(g1));
    }
```

### Step 3: labeled const nested inside a `Call` argument

```rust
    #[test]
    fn call_wrapping_labeled_const_arg_surfaces_in_labeled_consts() {
        let spec = notation::parse("floor(3[dex] + 2)", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.labeled_consts.len(), 1);
        assert_eq!(out.labeled_consts[0].value, 3);
        assert_eq!(out.labeled_consts[0].label, Some("dex".to_string()));
    }
```

### Verification

Run the full Rust CI gate battery. Dispatch both `shadowcat-spec-reviewer` and
`shadowcat-code-reviewer` on this task's diff (same heavier tier as Task 1 — this task's tests are
the evidence that Task 1's cursor-threading is correct).

---

## Task 3: `tr<offset>[:<value>][<label>]` tier-ladder modifier

**Files:**
- Modify: `src/server/src/dice/notation/lexer.rs` (add `Token::Colon`, lexer test)
- Modify: `src/server/src/dice/notation/parser.rs` (`P.tiers` scratch field, `tr` modifier arm,
  `Mode` construction threading, tests)
- Modify: `src/server/src/chat/rolls.rs` (stale-comment cleanup on `validate_tiers`, test proving
  the duplicate-offset rejection is now reachable from notation)

**Design note (spec gap, resolved via existing precedent, not silently assumed):** the spec's
`tr<offset>` grammar does not state whether `offset` may be negative. No existing notation
modifier supports a negative literal threshold (`cs>-5`, `t-3`, etc. are equally unsupported —
`expect_int()` only ever reads a bare `Token::Int`, never a preceding `Token::Minus`, at any
modifier site). `tr<offset>` follows the same existing convention: `margin_offset` is read via
`expect_int()`, so only non-negative offsets are notation-authorable in v1, consistent with every
other modifier threshold in this grammar. `Tier.margin_offset: i32` itself is unconstrained
(hand-built `RollSpec`s may still use negative offsets), so this is a notation-surface scope
boundary, not a data-model limitation.

### Step 1: `Token::Colon` in `lexer.rs`

Add a new `Token` variant (after `Comma`, added in Task 1):

```rust
    /// `,` — separates arguments in a `fn_call`.
    Comma,
    /// `:` — separates a modifier's threshold from its optional value fields
    /// (`tr<offset>:<value>`, `xs<N>:<extra>:<counter>`, `xf<N>:<lost>:<counter>`).
    Colon,
```

Add the `Display` arm:

```rust
            Token::Comma => write!(f, "','"),
            Token::Colon => write!(f, "':'"),
```

Add the lexing arm, alongside the comma arm added in Task 1:

```rust
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            ':' => {
                out.push(Token::Colon);
                i += 1;
            }
```

Add a lexer test:

```rust
    #[test]
    fn lex_colon_separates_modifier_value_fields() {
        let toks = lex("tr3:1").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Ident("tr".into()),
                Token::Int(3),
                Token::Colon,
                Token::Int(1),
            ]
        );
    }
```

### Step 2: `P.tiers` scratch field and `tr` modifier arm in `parser.rs`

Update the import line to add `Tier`:

```rust
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, DiceGroup, DieKind, Direction, ExplodeKind, Expr, FnName,
    GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, Tier, TotalConfig,
};
```

Add a field to `struct P`:

```rust
    /// Roll-level expertise budget from an `e<N>` token. Shared state,
    /// not per-`DiceGroup`; applied only when the resolved mode is SuccessCount.
    expertise: Option<u32>,
    /// Tier-ladder rungs accumulated from `tr<offset>[:<value>][<label>]` modifiers, in
    /// occurrence order. Threaded into whichever `TotalConfig.tiers`/`SuccessConfig.tiers` the
    /// resolved mode builds; empty means the default 2-rung pass/fail ladder
    /// (`eval::classify::classify`). Repeatable — unlike `success`/`t_target`/`expertise`, a
    /// second `tr` is not an error; `chat::rolls::validate_tiers` rejects a duplicate
    /// `margin_offset` at the wire boundary, not this parser.
    tiers: Vec<Tier>,
```

Update the `P { ... }` literal in `parse()`:

```rust
    let mut p = P {
        toks,
        pos: 0,
        success: None,
        t_target: None,
        expertise: None,
        tiers: Vec::new(),
    };
```

Update both `Mode` construction arms at the end of `parse()`:

```rust
        Mode::SuccessCount(SuccessConfig {
            success: rule,
            required_successes: None,
            tiers: p.tiers,
            crit_success: None,
            crit_fail: None,
            expertise: p.expertise.unwrap_or(0),
        })
    } else {
        Mode::Total(TotalConfig {
            difficulty: p.t_target,
            tiers: p.tiers,
        })
    };
```

(`p.crit_success`/`p.crit_fail` here stay `None` until Task 4 replaces them with `p.crit_success`/
`p.crit_fail` scratch-field reads.)

Add a `"tr"` arm to the `match id.as_str()` block in `modifiers()`, after the existing `"cf"` arm
and before the catch-all `other => ...`:

```rust
                        "tr" => {
                            let margin_offset = self.expect_int()?;
                            let tier_value = if matches!(self.peek(), Some(Token::Colon)) {
                                self.bump();
                                Some(self.expect_int()?)
                            } else {
                                None
                            };
                            let label = self.take_label();
                            self.tiers.push(Tier {
                                margin_offset,
                                label,
                                tier_value,
                            });
                        }
```

### Step 3: parser tests in `parser.rs`

```rust
    #[test]
    fn parses_single_tier_rung_with_value_and_label() {
        let spec = parse("4d6cs>4tr3:1[Good]", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.tiers,
                vec![Tier {
                    margin_offset: 3,
                    label: Some("Good".into()),
                    tier_value: Some(1)
                }]
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn parses_two_tier_rungs_appended_in_order() {
        let spec = parse("4d6cs>4tr3:1[Good]tr6:2[Great]", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.tiers,
                vec![
                    Tier {
                        margin_offset: 3,
                        label: Some("Good".into()),
                        tier_value: Some(1)
                    },
                    Tier {
                        margin_offset: 6,
                        label: Some("Great".into()),
                        tier_value: Some(2)
                    },
                ]
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn tr_value_and_label_are_optional() {
        let spec = parse("1d20t10tr5", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::Total(c) => assert_eq!(
                c.tiers,
                vec![Tier {
                    margin_offset: 5,
                    label: None,
                    tier_value: None
                }]
            ),
            other => panic!("expected Total, got {other:?}"),
        }
    }

    #[test]
    fn no_tr_leaves_tiers_empty() {
        let spec = parse("1d20t10", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::Total(c) => assert!(c.tiers.is_empty()),
            other => panic!("expected Total, got {other:?}"),
        }
    }
```

### Step 4: `validate_tiers` stale-comment cleanup and reachability test in `chat/rolls.rs`

Replace the doc comment on `validate_tiers` (currently claims notation cannot author a non-empty
ladder, which becomes false once this task lands):

```rust
/// Uniqueness guard over a classification ladder's `margin_offset`s. Reachable from untrusted
/// notation via the `tr<offset>[:<value>][<label>]` modifier (`dice::notation::parser`'s `"tr"`
/// arm), which can append a duplicate offset; `classify::classify`'s `max_by_key`/`min_by_key`
/// tie on a duplicate `margin_offset` is caller-order-dependent (documented on
/// `dice::eval::classify`), so a malformed ladder with a repeated offset would otherwise resolve
/// nondeterministically.
fn validate_tiers(tiers: &[crate::dice::spec::Tier]) -> Result<(), RollError> {
```

Add a test proving the reachability claim, near `duplicate_tier_offsets_are_rejected_pre_roll`:

```rust
    #[test]
    fn duplicate_tr_offsets_from_notation_are_rejected_at_the_wire_boundary() {
        match validate_formula("4d6cs>4tr3:1[Good]tr3:2[Also]", total_ctx()) {
            Err(RollError::DuplicateTierOffset(3)) => {}
            other => panic!("expected DuplicateTierOffset(3), got {other:?}"),
        }
    }
```

### Verification

Run the full Rust CI gate battery.

---

## Task 4: `xs`/`xf` crit-trigger modifiers + `NOTATION_KEYWORDS` parity (same commit)

**Files:**
- Modify: `src/server/src/dice/notation/parser.rs` (`P.crit_success`/`P.crit_fail` scratch
  fields, `optional_colon_int` helper, `xs`/`xf` modifier arms, `Mode` construction threading,
  two new `ParseError` variants, tests)
- Modify: `src/server/src/dice/notation/mod.rs` (`DuplicateCritSuccess`/`DuplicateCritFail`
  variants + `Display` + updated variant-count test)
- Modify: `src/client/formula/src/template.ts` (`NOTATION_KEYWORDS` gains `"tr"`, `"xs"`, `"xf"`)
- Modify: `src/server/src/chat/rolls.rs` (one end-to-end integration test)

**Design note (spec ambiguity, resolved via existing precedent, not silently assumed):** the
spec's §4.2 says `xs`/`xf` get "the same mode-gating `cs`/`cf`/`t`/`e` already have," but those four
modifiers span two different behaviors: `cs`/`cf` FORCE `SuccessCount` mode, while `t`/`e` are
ambient-dependent and SILENTLY DROPPED when the resolved mode ends up `Total` (the dice skill's
documented `e<N>`-under-Total-ambient gotcha). This plan resolves the ambiguity by mirroring `t`/`e`
specifically, not `cs`/`cf`: `xs`/`xf` set roll-level scratch fields consumed into
`SuccessConfig.crit_success`/`crit_fail` only when the resolved mode is `SuccessCount`, and are
silently dropped under `Total` — exactly `e<N>`'s existing, already-tested precedent. Forcing
`SuccessCount` from a crit-config-only modifier would make a lone `4d6xs20` (no `cs`/`t` present)
hard-error with "SuccessCount mode requires a per-die target," a confusing outcome for a plausible
partial notation string; mirroring `e<N>` avoids that and keeps the addition strictly additive.
Both directions are pinned by the tests in Step 3 below — a future implementer changing this
mode-gating must update `xs_and_xf_under_total_ambient_are_silently_dropped` deliberately, not by
accident.

### Step 1: `P` scratch fields, helper, and modifier arms in `parser.rs`

Update the import line to add `CritFail`, `CritSuccess`, `CritTrigger`:

```rust
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, CritFail, CritSuccess, CritTrigger, DiceGroup, DieKind,
    Direction, ExplodeKind, Expr, FnName, GroupModifier, Mode, RollSpec, SuccessConfig,
    SuccessRule, Tier, TotalConfig,
};
```

**Correction (post-Task-3 fix, applied to this section before Task 4 execution):** Task 3's own
review found `SuccessConfig.required_successes` had never been notation-settable — a
pre-existing, campaign-predating gap that left `tr<offset>`'s tier ladder permanently inert for
`SuccessCount` mode. Task 3 fixed this by adding an `rs<N>` modifier and a
`required_successes: Option<i32>` field on `struct P` (mirroring `expertise`'s exact pattern),
now already present in `parser.rs` ahead of Task 4. The snippets below are updated to carry that
field/value forward instead of the stale `required_successes: None` this section originally
specified — applying the ORIGINAL unpatched snippet would silently regress the Task 3 fix (parse
`rs<N>` successfully, populate `P.required_successes`, then discard it at `Mode` construction with
no error).

Add two fields to `struct P`, after `required_successes` (added by Task 3):

```rust
    /// Crit-success trigger from an `xs<N>[:<extra>[:<counter>]]` modifier. Shared roll-level
    /// state (a second `xs` errors via `ParseError::DuplicateCritSuccess` rather than silently
    /// overwriting); consumed into `SuccessConfig.crit_success` only when the resolved mode is
    /// `SuccessCount`, mirroring `expertise`'s silent-drop under `Total`.
    crit_success: Option<CritSuccess>,
    /// Crit-fail trigger from an `xf<N>[:<lost>[:<counter>]][!]`. Same sharing/mode-gating as
    /// `crit_success`.
    crit_fail: Option<CritFail>,
```

Update the `P { ... }` literal in `parse()` (already carries `required_successes: None` from
Task 3 — leave that line as-is, add the two new fields):

```rust
    let mut p = P {
        toks,
        pos: 0,
        success: None,
        t_target: None,
        expertise: None,
        required_successes: None,
        tiers: Vec::new(),
        crit_success: None,
        crit_fail: None,
    };
```

Update the `SuccessCount` `Mode` construction arm at the end of `parse()` (already reads
`required_successes: p.required_successes` from Task 3 — leave that line as-is, add the two new
fields):

```rust
        Mode::SuccessCount(SuccessConfig {
            success: rule,
            required_successes: p.required_successes,
            tiers: p.tiers,
            crit_success: p.crit_success,
            crit_fail: p.crit_fail,
            expertise: p.expertise.unwrap_or(0),
        })
```

Add a new method on `impl P`, after `cmp_target_required`:

```rust
    /// Reads an optional `:<int>` suffix, defaulting to `default` when no `Colon` token is
    /// present. Shared by `xs`/`xf`'s repeated `:<value>[:<value>]` shape: calling this twice in
    /// a row correctly reads zero, one, or two colon-prefixed values in sequence.
    fn optional_colon_int(&mut self, default: i32) -> Result<i32, ParseError> {
        if matches!(self.peek(), Some(Token::Colon)) {
            self.bump();
            self.expect_int()
        } else {
            Ok(default)
        }
    }
```

Add `"xs"` and `"xf"` arms to `modifiers()`'s `match id.as_str()` block, after the `"tr"` arm added
in Task 3 and before the catch-all:

```rust
                        "xs" => {
                            if self.crit_success.is_some() {
                                return Err(ParseError::DuplicateCritSuccess);
                            }
                            let threshold = self.expect_int()?;
                            let extra_successes = self.optional_colon_int(1)?;
                            let positive_counter = self.optional_colon_int(1)?;
                            self.crit_success = Some(CritSuccess {
                                trigger: CritTrigger::AtLeast(threshold),
                                extra_successes,
                                positive_counter,
                            });
                        }
                        "xf" => {
                            if self.crit_fail.is_some() {
                                return Err(ParseError::DuplicateCritFail);
                            }
                            let threshold = self.expect_int()?;
                            let lost = self.optional_colon_int(1)?;
                            let negative_counter = self.optional_colon_int(1)?;
                            let allow_negative = matches!(self.peek(), Some(Token::Bang));
                            if allow_negative {
                                self.bump();
                            }
                            self.crit_fail = Some(CritFail {
                                trigger: CritTrigger::AtLeast(threshold),
                                lost,
                                negative_counter,
                                allow_negative,
                            });
                        }
```

### Step 2: `ParseError` additions in `notation/mod.rs`

Add two variants to the `ParseError` enum, after `DuplicateExpertise`:

```rust
    /// A second `e<N>` expertise token appeared in one roll. `expertise` is shared
    /// roll-level parser state (one `RollSpec`), so a silent overwrite would lose one.
    DuplicateExpertise,
    /// A second `xs<N>` crit-success trigger appeared in one roll. Shared roll-level state (one
    /// `SuccessConfig.crit_success`), so a silent overwrite would lose one.
    DuplicateCritSuccess,
    /// A second `xf<N>` crit-fail trigger appeared in one roll. Same reasoning as
    /// `DuplicateCritSuccess`.
    DuplicateCritFail,
```

Add the matching `Display` arms, after `DuplicateExpertise`'s:

```rust
            ParseError::DuplicateExpertise => {
                write!(f, "a roll can only set one expertise budget (e<N>)")
            }
            ParseError::DuplicateCritSuccess => {
                write!(f, "a roll can only set one crit-success trigger (xs<N>)")
            }
            ParseError::DuplicateCritFail => {
                write!(f, "a roll can only set one crit-fail trigger (xf<N>)")
            }
```

Update `every_parse_error_variant_displays_without_debug_artifacts`'s `variants` vec and count
(**10 → 12** — corrected from this section's original 9 → 11: Task 3 already added
`ParseError::DuplicateRequiredSuccesses`, bringing the pre-Task-4 count to 10, not 9):

```rust
        let variants: Vec<ParseError> = vec![
            ParseError::Empty,
            ParseError::Unexpected("expected a number, found the number 5".to_string()),
            ParseError::Trailing("the number 5".to_string()),
            ParseError::InvalidDieSides(0),
            ParseError::DuplicateSuccessRule,
            ParseError::DuplicateExpertise,
            ParseError::DuplicateRequiredSuccesses,
            ParseError::EmptyLabel,
            ParseError::UnterminatedLabel,
            ParseError::InvalidLabelChar,
            ParseError::DuplicateCritSuccess,
            ParseError::DuplicateCritFail,
        ];
        assert_eq!(
            variants.len(),
            12,
            "update this test if a ParseError variant is added or removed"
        );
```

Add one new entry to `real_parse_failures_render_without_debug_artifacts`'s `inputs` array
(already carries Task 3's `"4d6cs>=4rs2rs3"` entry — leave that as-is, add this one):

```rust
            "min(3)",                   // fn_call: wrong arity
            "foo(3)",                   // fn_call: unknown function name
            "4d6cs>=4rs2rs3",           // duplicate rs
            "4d6cs>=4xs5xs6",           // duplicate xs
```

### Step 3: parser tests in `parser.rs`

```rust
    #[test]
    fn parses_xs_with_defaults() {
        let spec = parse("4d6cs>=4xs20", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.crit_success,
                Some(CritSuccess {
                    trigger: CritTrigger::AtLeast(20),
                    extra_successes: 1,
                    positive_counter: 1
                })
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn parses_xs_with_explicit_extra_and_positive_counter() {
        let spec = parse("4d6cs>=4xs20:3:2", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.crit_success,
                Some(CritSuccess {
                    trigger: CritTrigger::AtLeast(20),
                    extra_successes: 3,
                    positive_counter: 2
                })
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_xs_errors() {
        assert!(matches!(
            parse("4d6cs>=4xs20xs19", ParseContext::default()),
            Err(ParseError::DuplicateCritSuccess)
        ));
    }

    #[test]
    fn parses_xf_with_defaults() {
        let spec = parse("4d6cs>=4xf1", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.crit_fail,
                Some(CritFail {
                    trigger: CritTrigger::AtLeast(1),
                    lost: 1,
                    negative_counter: 1,
                    allow_negative: false
                })
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn parses_xf_with_bang_sets_allow_negative() {
        let spec = parse("4d6cs>=4xf1!", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert!(c.crit_fail.unwrap().allow_negative),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_xf_errors() {
        assert!(matches!(
            parse("4d6cs>=4xf1xf2", ParseContext::default()),
            Err(ParseError::DuplicateCritFail)
        ));
    }

    #[test]
    fn xs_and_xf_under_total_ambient_are_silently_dropped() {
        // Mirrors e<N>'s exact silent-drop-under-Total precedent: xs/xf set roll-level
        // scratch fields that are only consumed when the resolved mode is SuccessCount.
        let spec = parse("1d20t10xs15xf1", ParseContext::default()).unwrap(); // ambient Total
        match spec.mode {
            Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
            other => panic!("expected Total, got {other:?}"),
        }
    }
```

### Step 4: `NOTATION_KEYWORDS` parity update in `template.ts`

In `src/client/formula/src/template.ts`, update the `NOTATION_KEYWORDS` export (already carries
Task 3's `"tr"`/`"rs"` entries — leave those as-is, append `"xs"`/`"xf"`):

```typescript
export const NOTATION_KEYWORDS: readonly string[] = [
  DICE_OPERATOR, "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e", "tr", "rs", "xs", "xf",
];
```

No other change to `template.ts` is needed: `scripts/check-notation-modifier-parity.mjs`'s
`extractRustModifierIdents` reads the `"tr"`/`"xs"`/`"xf"` arm heads directly out of `parser.rs`'s
`match id.as_str()` block (the same block Tasks 3–4 added arms to), so the parity check picks up
the new modifiers automatically once both sides land in this commit.

### Step 5: end-to-end integration test in `chat/rolls.rs`

```rust
    #[test]
    fn xs_modifier_from_notation_fires_crit_success_end_to_end() {
        let spec = crate::dice::notation::parse("6d6cs>=4xs6", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = eval::evaluate(&spec, &raws);
        let expected_crits = raws
            .records
            .iter()
            .filter(|r| r.kept && r.value >= 6)
            .count() as i32;
        assert_eq!(out.crit_successes, expected_crits);
    }
```

### Verification

Run the full Rust CI gate battery AND the full client gate battery (Global Constraints items 6–8)
— this is the only task in this plan that touches `src/client/**`. Confirm
`pnpm run test:scripts` passes, which mechanically proves `NOTATION_KEYWORDS` and `P::modifiers`
agree.

---

## Task 5: Codebase-skill doc-sync, spec-review dispatch, plugin version bump, `TODO.md` cleanup

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-dice/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-formula/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: `.claude/.claude-plugin/plugin.json`
- Modify: `docs/TODO.md`

### Step 1: `shadowcat-codebase-dice` SKILL.md updates

Edit 1 — update the grammar snippet and add `Expr::Call`/`FnName` + the two new tokens to the
`dice::notation` bullet under **Key files & seams**. Find:

```
- `dice::notation` (its `lexer` and `parser` submodules) — `lex`/`Token`/`ParseError` +
  `parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError>` (recursive descent:
  `expr := term (('+'|'-') term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')'
  | '-' factor | dice | int`). `ParseContext{mode: ModeKind, direction: Direction}` is caller-
```

Replace with:

```
- `dice::notation` (its `lexer` and `parser` submodules) — `lex`/`Token`/`ParseError` +
  `parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError>` (recursive descent:
  `expr := term (('+'|'-') term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')'
  | '-' factor | fn_call | dice | int`, `fn_call := ident '(' expr (',' expr)* ')'`). `fn_call`
  recognizes an `Ident` as a math function only when immediately followed by `(` at the `factor`
  position — the same lexer `Ident` token the modifier grammar already consumes, so no lexer
  change was needed for the six function names themselves; `FnName::arity` is checked at parse
  time, producing `Expr::Call{name: FnName, args}`. `Token::Comma`/`Token::Colon` are two new
  single-char tokens: `Comma` separates `fn_call` arguments, `Colon` separates a modifier's
  threshold from its optional value fields (`tr<offset>:<value>`, `xs<N>:<extra>:<counter>`,
  `xf<N>:<lost>:<counter>`). `ParseContext{mode: ModeKind, direction: Direction}` is caller-
```

Edit 2 — the crit-events gotcha, currently stale after `xs`/`xf` land. Find:

```
- **The notation-level `cs>N`/`cf<N` tokens and
  `SuccessConfig.crit_success`/`crit_fail` are two unrelated mechanisms that happen to share
  initials.** `cs>N`/`cf<N` in a dice-notation string set
  the ordinary per-die `SuccessRule` (or its inverted-comparator `cf<N`
  approximation);
  they do NOT construct a `CritSuccess`/`CritFail` struct. Today, crit events are configurable
  only by authoring a `RollSpec`/`SuccessConfig` directly — no notation syntax exposes them yet.
```

Replace with:

```
- **The notation-level `cs>N`/`cf<N` tokens and
  `SuccessConfig.crit_success`/`crit_fail` are two unrelated mechanisms that happen to share
  initials.** `cs>N`/`cf<N` in a dice-notation string set
  the ordinary per-die `SuccessRule` (or its inverted-comparator `cf<N`
  approximation); they do NOT construct a `CritSuccess`/`CritFail` struct. Those come from the
  SEPARATE `xs<N>[:<extra>[:<counter>]]`/`xf<N>[:<lost>[:<counter>]][!]` modifiers, which set
  `SuccessConfig.crit_success`/`crit_fail` directly — `CritTrigger::AtLeast` only (v1 notation has
  no syntax for `CritTrigger::HasSymbol`, a deliberate scope boundary: it names an opaque,
  system-defined `Symbol`, and this crate carries zero game-system vocabulary). `xs`/`xf` are
  roll-level parser scratch state (`P.crit_success`/`P.crit_fail`) consumed into `SuccessConfig`
  only when the resolved mode is `SuccessCount` — silently dropped under `Total`, mirroring
  `e<N>`'s existing mode-gating precedent (see the `e<N>` gotcha below) rather than FORCING
  `SuccessCount` the way `cs`/`cf` do.
```

Edit 3 — the `validate_tiers` gotcha, currently stale after `tr` lands. Find:

```
- **`validate_tiers` (`chat::rolls`) guards `SuccessConfig`/`TotalConfig.tiers`
  uniqueness at the wire boundary**, ahead of any untrusted construction path existing —
  `classify::classify`'s `max_by_key`/`min_by_key` tie on a duplicate `margin_offset` is
  caller-order-dependent (documented on `dice::eval::classify`), so a malformed ladder with a
  repeated offset would otherwise resolve nondeterministically. `validate_pre_roll` calls it on
  every parsed spec's tiers; `RollError::DuplicateTierOffset(i32)` is the player-presentable
  rejection. Notation still cannot author a non-empty ladder today (`dice::notation::parser`
  emits `tiers: vec![]`), so this guard arms the boundary before the construction path exists,
  mirroring the `DieKind::validate()` precedent above.
```

Replace with:

```
- **`validate_tiers` (`chat::rolls`) guards `SuccessConfig`/`TotalConfig.tiers`
  uniqueness at the wire boundary** —
  `classify::classify`'s `max_by_key`/`min_by_key` tie on a duplicate `margin_offset` is
  caller-order-dependent (documented on `dice::eval::classify`), so a malformed ladder with a
  repeated offset would otherwise resolve nondeterministically. `validate_pre_roll` calls it on
  every parsed spec's tiers; `RollError::DuplicateTierOffset(i32)` is the player-presentable
  rejection. Reachable from untrusted notation via the repeatable `tr<offset>[:<value>][<label>]`
  modifier (`dice::notation::parser`'s `"tr"` arm appends one `Tier` rung per occurrence, with no
  parse-time duplicate check of its own — `validate_tiers` is the sole enforcement point, exactly
  the `DieKind::validate()` precedent above).
```

Edit 4 — the `dice::spec` bullet's AST-type list, to mention `FnName`/`Expr::Call`. Find the
sentence ending `` `Expr` (Dice(DiceGroup)/Const(ConstTerm)/Bin/Neg). `` and replace with:
`` `Expr` (Dice(DiceGroup)/Const(ConstTerm)/Bin/Neg/Call{name: FnName, args}) — `FnName` (Floor/
Ceil/Round/Abs/Min/Max, `arity()` fixed per variant, checked at parse time only). ``

Edit 5 — the notation-modifier-vocabulary Hard Invariant already states the parity mechanism
generically and needs no wording change, but append one clarifying sentence about the math-function
scoping boundary at its end (spec §4.3's resolved fork, worth pinning for future readers):

```
  ... a wrong roll seen by whoever authored the template. **The six math-function names
  (`floor`/`ceil`/`round`/`abs`/`min`/`max`) are deliberately NOT part of this parity set** — they
  never enter `P::modifiers`'s match at all (`fn_call` is a separate `factor`-level grammar
  production), so `modifierParityDifference` never sees them; `@shadowcat/formula` reserves the
  same six names independently as its own function set, a coincidental alignment rather than a
  parity-enforced one.
```

### Step 2: `shadowcat-codebase-formula` SKILL.md updates

Add one clarifying sentence to the "dice-modifier vocabulary" gotcha bullet (find `Without that
gate the only signal is a wrong roll, seen by whoever wrote the template.` and insert immediately
after):

```
  **Math-function names (`floor`/`ceil`/`round`/`abs`/`min`/`max`) do NOT belong in
  `NOTATION_KEYWORDS`** — it guards the dice-MECHANIC modifier vocabulary specifically (the same
  category as `kh`/`cs`/`tr`/`xs`/`xf`), not every token the notation grammar's `fn_call`
  production recognizes; this package reserves the same six names independently as its own
  function set (`FN_NAMES`/`FnName` in `parser`), a coincidental alignment, not a
  parity-enforced one.
```

### Step 3: `shadowcat-codebase-chat` SKILL.md update

In the "Dice wire" section, find:

```
- `chat::rolls`: caps (`MAX_ROLL_DICE=100` summed over the parsed `Expr`; `MAX_ROLL_RECORDS=1000`
```

Replace with:

```
- `chat::rolls`: caps (`MAX_ROLL_DICE=100` summed over the parsed `Expr` — `walk_groups` recurses
  into `Expr::Call`'s arguments too, so a dice group nested inside a math-function call still
  counts; `MAX_ROLL_RECORDS=1000`
```

### Step 4: dispatch `shadowcat-spec-reviewer`

Dispatch `shadowcat-spec-reviewer` on the three skill diffs from Steps 1–3, confirming each
accurately captures the change with no omission, drift, or broken pointer. Record the PASS/FAIL
result; if FAIL, fix and re-dispatch before proceeding.

### Step 5: plugin version bump

In `.claude/.claude-plugin/plugin.json`, bump `"version"` from `"1.0.59"` to `"1.0.60"`.

### Step 6: `docs/TODO.md` cleanup

In `docs/TODO.md`, under the `## Follow-on feature sub-projects (own brainstorm → spec → plan
each)` heading, remove numbered item 4 (the dice-notation grammar growth entry this plan
implements) and renumber items 5–8 down by one to close the gap. Find:

```
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Dice-notation grammar growth** — math fns (floor/ceil/round/abs/min/max) + crit-event /
   tier-ladder notation syntax.
5. **Per-channel / per-message dice-settings overrides** — needs a channel model.
6. **In-body doc-link chat segment** (`Segment::DocLink`) — actor-name → sheet navigation shipped
   in M12c, but a free-form doc-link segment has no server producer or client authoring path yet;
   needs a server producer + authoring affordance.
7. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` is REJECTED at ingest (fail-closed,
   no first-party producer) — build the composer/token-context UX and lift the rejection together.
8. **Real-time per-recipient move-streaming** — `MoveStream` precomputes each move's
```

Replace with:

```
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Per-channel / per-message dice-settings overrides** — needs a channel model.
5. **In-body doc-link chat segment** (`Segment::DocLink`) — actor-name → sheet navigation shipped
   in M12c, but a free-form doc-link segment has no server producer or client authoring path yet;
   needs a server producer + authoring affordance.
6. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` is REJECTED at ingest (fail-closed,
   no first-party producer) — build the composer/token-context UX and lift the rejection together.
7. **Real-time per-recipient move-streaming** — `MoveStream` precomputes each move's
```

The remainder of the old item 8's body (the three lines following, ending "...replacing
execute-time precompute.") is unchanged — only its leading number moves from `8.` to `7.`; re-check
`docs/TODO.md`'s live content before editing in case another task has touched this list first
(number the remaining items by what is actually on disk at edit time, not by the literal digits
quoted here, if they have drifted since this plan was written).

### Verification

Confirm `git diff --stat` for this task touches only the five files listed above. No CI gate
battery applies to Task 5 (skill/doc/config files only, not `.rs`/`.ts` source).
