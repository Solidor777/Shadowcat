# Dice-Notation Grammar Growth — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority).

**Spec for:** `docs/TODO.md` bucket-C sub-project 4, "Dice-notation grammar growth — math fns
(floor/ceil/round/abs/min/max) + crit-event / tier-ladder notation syntax."

## 1. Scope

Two independent additions to `dice::notation`:

1. **Math functions** (`floor`, `ceil`, `round`, `abs` — unary; `min`, `max` — binary) as a new
   grammar production, evaluated by a new `Expr::Call` AST variant.
2. **Crit-event and tier-ladder notation syntax** for the already-fully-built-and-evaluated
   `Tier`/`CritSuccess`/`CritFail` data model — `dice::eval::classify`/`dice::eval::crit` need
   zero changes; only `dice::notation::parser` gains new modifier keywords and `RollSpec`
   construction stops hardcoding `tiers: vec![]`.

Both are additive grammar extensions; no existing notation syntax changes meaning.

## 2. Why math functions belong in notation, not just in formula — resolved design fork

`@shadowcat/formula` already has its own `floor`/`ceil`/`round`/`abs`/`min`/`max` (a separate
grammar/evaluator that a document template resolves BEFORE the result is substituted into a
notation string the server parses). This is not redundant with adding the same functions to
dice-notation itself: a player typing a roll directly into chat (`/roll floor(1d20/2)`) never goes
through formula templating at all — formula only applies to document-bound authored templates.
Notation is the grammar reachable from a bare typed roll command, so it needs its own function
support to serve that path. The two implementations stay genuinely separate, per the dice/formula
skills' existing "two different grammars" invariant — this sub-project does not unify them.

## 3. Math functions — grammar & evaluator

### 3.1 AST

```rust
pub enum FnName { Floor, Ceil, Round, Abs, Min, Max }

pub enum Expr {
    Dice { .. },
    Const { .. },
    Bin { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Neg(Box<Expr>),
    Call { name: FnName, args: Vec<Expr> },   // new
}
```

Arity is fixed per `FnName` and checked at parse time (mirroring `@shadowcat/formula`'s own
`checkArity` pattern, independently reimplemented — no shared code, per §2): `Floor`/`Ceil`/
`Round`/`Abs` take exactly 1 arg, `Min`/`Max` take exactly 2. A wrong count is a `ParseError`
naming the function and the expected/actual arity.

### 3.2 Grammar

```
factor := '-' factor | '(' expr ')' | fn_call | int 'd' int modifiers | int [label]
fn_call := ident '(' expr (',' expr)* ')'
```

`ident` here is the same lexer `Ident` token the modifier grammar already consumes — recognized as
a function call only when immediately followed by `(` at the `factor` position (the ONLY place an
`Ident` can appear outside `modifiers` today), so no lexer change is needed, only a new parser
branch at `factor`. An `ident` at this position that isn't one of the six recognized names is a
`ParseError::Unexpected("unknown function '{name}'")`, mirroring the existing unknown-modifier
error shape.

### 3.3 Evaluation — extends the existing group-index-cursor walk, not a new mechanism

`eval::sum::fold` and `eval::eval::roll()` both recurse through `Expr` today; `Call` gets a new
match arm in both that folds/evaluates each `args[i]` **in left-to-right order, threading the same
`group_index` cursor sequentially through each argument** — exactly how `Bin{lhs, rhs}` already
threads the cursor through two children today, generalized to N children. This is not a new
correctness risk category: it's the same recursive pattern the codebase's dice skill already
identifies as correctness-critical, extended along an axis (child count) the existing `Bin` case
already established. Each argument evaluates to a plain number (dice inside it are fully resolved
and folded first, exactly as they are inside any arithmetic sub-expression today); the function is
then applied to those numbers. A `Call` node itself introduces no new dice groups and consumes no
group-index slots beyond what its arguments consume.

`Min`/`Max` compare the two evaluated argument values; `Floor`/`Ceil`/`Round` apply the
corresponding `f64` op then cast back through the same int-rounding convention the rest of
`eval::sum` already uses for a group's total; `Abs` is `.abs()`. No new numeric type is introduced.

## 4. Crit/tier notation syntax

### 4.1 Tier-ladder modifier: `tr<offset>[:<value>][<label>]`

One `tr` occurrence adds one `Tier{margin_offset, tier_value, label}` rung. Repeatable (each
occurrence on the same dice group appends another rung to that roll's `tiers: Vec<Tier>>`).
`:<value>` is optional (`tier_value: Option<i32>`); the existing bracket-label syntax
(`[text]`, already grammar-legal trailing on any atomic factor) is reused verbatim for the rung's
`label` rather than inventing a second label syntax — this is the same "state the constraint,
reuse the existing mechanism" reasoning applied to grammar, not just prose. Example:
`4d6cs>4tr3:1[Good]tr6:2[Great]` — two rungs on a `SuccessCount`-mode roll.
`validate_tiers`'s existing duplicate-`margin_offset` rejection is unchanged and now reachable from
notation (previously only reachable by hand-constructing a `RollSpec`).

### 4.2 Crit-trigger modifiers: `xs<N>[:<extra_successes>[:<positive_counter>]]` and
`xf<N>[:<lost>[:<negative_counter>]][!]`

`xs`/`xf` set `SuccessConfig.crit_success`/`crit_fail` (only meaningful in `SuccessCount` mode,
same mode-gating `cs`/`cf`/`t`/`e` already have). `<N>` is the `CritTrigger::AtLeast(N)` threshold
— **v1 notation supports `AtLeast` triggers only; `CritTrigger::HasSymbol` gets no notation syntax
in this sub-project.** This is a deliberate scope boundary, not an oversight: `HasSymbol` names an
opaque, system-defined `Symbol` string, and the dice crate's Hard Invariant is "zero game-system
vocabulary" — there is no existing notation precedent for embedding an arbitrary opaque string
token, and designing one is a separate, independently-scoped question (a generic `sym:"name"`
syntax, quoting rules, etc.) that the TODO's one-line "crit-event notation syntax" ask does not
by itself require resolving. `extra_successes`/`positive_counter` (for `xs`) and `lost`/
`negative_counter` (for `xf`) default to `1` when the colon-suffix is omitted — the common case
("a 20 is worth one extra success") needs only `xs20`. `xf`'s trailing `!` sets
`CritFail.allow_negative = true`; its absence leaves the existing default.

### 4.3 `NOTATION_KEYWORDS` parity — resolved design fork

`tr`, `xs`, `xf` are genuine new dice-notation MODIFIER keywords (the same category as `kh`/`cs`/
`t`), so they go into `@shadowcat/formula`'s `NOTATION_KEYWORDS` parity set in the same commit —
the existing `modifierParityDifference` test governs them exactly as it governs every existing
modifier. **The six math-function names do NOT go into `NOTATION_KEYWORDS`.** That parity set
exists so `@shadowcat/formula` can recognize a reserved dice-mechanic word appearing inside a raw
notation span it isn't itself parsing — it is scoped to the dice-MECHANIC modifier vocabulary, not
to every token dice-notation's grammar happens to recognize. Widening it to also cover math
functions would be applying that mechanism outside what it guards: `@shadowcat/formula` already
independently reserves `floor`/`ceil`/`round`/`abs`/`min`/`max` as ITS OWN function names (per §2,
a coincidental, not parity-enforced, alignment) — there is no shared decision here for a parity
test to protect, only two grammars that happen to use the same math vocabulary because it's the
obvious vocabulary for the job.

## 5. Testing

Per-keyword/per-function unit tests at the same one-case-per-behavior granularity `parser.rs`'s
existing ~330 lines already establish: valid syntax, arity errors, unknown-function-name error,
duplicate-tier-offset rejection reachable from notation, `xs`/`xf` default-value omission, `xf`'s
`!` flag, and at least one `Call` node wrapping a dice group (`floor(1d20/2)`) verified against
`eval::sum::fold`'s group-boundary reconstruction directly (assert the underlying d20's individual
result is still recoverable in the roll's record, not just the floored total) — this is the one
correctness-critical case per §3.3 and gets the heavier review tier the dice skill already mandates
for `eval::sum`/`eval::classify`/`eval::crit`-touching changes.

## 6. Non-goals

- `CritTrigger::HasSymbol` notation syntax (§4.2).
- Any change to `@shadowcat/formula`'s own math-function implementation.
- Any change to `eval::classify`/`eval::crit`/`eval::success` themselves — the data model is
  already correct; only the parser's construction of it changes.
