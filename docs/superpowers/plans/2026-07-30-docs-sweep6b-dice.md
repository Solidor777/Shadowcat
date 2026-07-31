# Docs Sweep 6b — Dice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/dice/` — measured backlog 172 (spec.rs 73,
outcome.rs 29, notation/lexer.rs 16, notation/parser.rs 11, notation/mod.rs
10, eval/crit.rs 8, mod.rs 6, eval/mod.rs 6, rng.rs 4, eval/sum.rs 3,
eval/classify.rs 3, recalc.rs 2, eval/expertise.rs 1; proptests.rs,
eval/groups.rs, eval/success.rs already clean) — then flip the whole dice/
tree (all 16 files) to deny. This closes the LAST undocumented server scope.

**Architecture:** Same calibrated pattern (prior sweep plans' Global
Constraints verbatim). Branch `docs-sweep6b-dice`. Ship with the LOCAL
matrix. Reviews under the no-shell protocol (pre-generated diff + relayed
evidence; reviewers must not run `cargo test`).

**Truthfulness hot spots:** NEVER document a notation marker, token, or
grammar rule from memory — quote the lexer/parser's enforcing line (the
Sweep-6a lesson: an invented `[[btn:...]]` marker survived to review); the
label grammar must be stated as `take_label()` enforces it TODAY — legal after
any atomic factor including a bare `Const` (the differential-e2e lesson's FIX;
never restate the pre-fix rule); determinism claims cite the seeded-RNG plumbing in
`rng.rs`/`recalc.rs` as implemented; saturation claims cite `eval::sum`'s
`*_saturating` helpers; crit/success ladder docs match `classify`'s
tie-refusal (the `RollError::AmbiguousLadder`-class rule documented in
rolls.rs) and `crit.rs`'s actual margin logic; cap constants cited by their
real names.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. No-shell final review
pair; fixes pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint attrs only). Standard final review only.

---

### Task 1: dice/spec.rs (73)

- [ ] Enumerate live (expect 73); document every item — the parsed-formula
  AST (expr/term/group/modifier types, per-variant + per-field), caps,
  mode/config structs. Doctests per policy (pure constructible types →
  runnable where public; ` ```text ` otherwise). Gates; commit.

### Task 2: dice/outcome.rs + dice/notation/{lexer,parser,mod}.rs (66)

- [ ] Enumerate live (expect 29+16+11+10); document — outcome/record wire
  types (client renders from these), token set QUOTED FROM THE LEXER, grammar
  rules quoted from the parser (incl. the label-after-DiceGroup-only rule).
  Doctests per policy. Gates; commit.

### Task 3: dice/{mod,rng,recalc}.rs + dice/eval/{mod,crit,sum,classify,expertise}.rs (33)

- [ ] Enumerate live (expect 6+4+2 and 6+8+3+3+1); document — module wiring,
  seeded RNG determinism, recalc entry, evaluator stages (crit margins,
  saturating sums, ladder classification, expertise). Doctests per policy.
  Gates; commit.

### Task 4: Deny flip + verify + sync + ship

- [ ] Inner deny pair in ALL 16 dice/ files (mod, outcome, proptests, recalc,
  rng, spec; notation/{lexer,mod,parser}; eval/{classify,crit,expertise,
  groups,mod,success,sum} — clean files get the attr too). Mutation proof on
  spec.rs + one eval file; restore via python. Full local matrix. Docs-sync:
  PLAN.md; dice skill ratchet Gotcha. No-shell review pair with pre-generated
  diff + relayed evidence; fix findings; merge `--ff-only`; push; delete
  branch; memory update (server tree fully deny-ratcheted except lib.rs —
  final-ratchet item).

---

## Deferred (logged, not dropped)

- Sweep 7+: client packages (core/render/ui-kit+shell/formula), then modules.
  Then buddy-check convergence → final ratchet → skills reference pass.
