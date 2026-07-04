# M11b-2 Expertise DP — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Scope:** M11b-2 only — the provably-optimal expertise allocator, in isolation. Highest-risk
piece of the whole dice engine.
**Builds on:** M11b-1 (merged to local main, stable). Refines §8 of the parent M11b design:
[`2026-07-04-m11b-system-rules-design.md`](2026-07-04-m11b-system-rules-design.md).
**Consumer:** M11d wires dice to transport/chat and adds ts-rs bindings + the untrusted-input
caps; **not** in scope here.

This document grounds §8's approved formulation against the *actual* M11b-1 code and resolves
four sub-questions §8 left open or specified wrongly. It does **not** re-litigate §8's settled
decisions: lexicographic objective, the `O(N·E²)` bounded DP, placement as a new `eval::expertise`
stage, `DieRecord.expertise` audit trail, symbolic-dice exclusion (moot in b-2 — `DieKind::Faces`
is b-3), or the mandatory brute-force differential oracle + buddy-check.

## 1. What M11b-1 already ships (the surface this extends)

The relevant stable shapes (read from source, not the parent spec's pre-b-1 snippets):

```rust
// spec.rs
struct SuccessConfig {
    success: SuccessRule,                 // per-die comparator+target; comp already encodes
    required_successes: Option<i32>,      //   direction (HighWins→Gte, LowWins→Lte at parse time)
    tiers: Vec<Tier>,
    crit_success: Option<CritSuccess>,
    crit_fail:    Option<CritFail>,
    // expertise: ABSENT — added by this checkpoint
}
// outcome.rs
struct DieRecord { id, group_index, natural, value, kept, exploded, rerolled_from,
                   crit_success: bool, crit_fail: bool /* , expertise: ADDED HERE */ }

// eval/crit.rs
fn score_die(direction, value, cfg: &SuccessConfig) -> DieCrit
//   → { is_success, is_fail, extra_successes, lost, positive_counter, negative_counter }
//   direction handled internally (`reaches` flips comparisons under LowWins).

// eval/success.rs — evaluate_success:
//   for each kept die: base += cfg.success.comp.test(value, target);
//                      dc = score_die(dir, value, cfg); fold extra/lost/pos/neg.
//   raw_net = base + extra − lost;  net = allow_negative ? raw_net : raw_net.max(0)
```

`roll` (only RNG step), `evaluate` (pure/deterministic — a **hard invariant**), and `recalculate`
keep their three-function contract.

## 2. Placement — a value-mutating pre-pass; b-1 counting stays sealed

Expertise runs as a new stage `eval::expertise`, called by `eval::success` **after** the kept dice
are pooled across all groups and **before** the existing success/crit counting loop. Its entire
effect on the pool is:

- **mutate** each chosen die's `DieRecord.value` to its optimal expertise-adjusted face, and
- **record** the points spent on that die in the new `DieRecord.expertise: i32`.

The existing `evaluate_success` counting loop then runs **unchanged** over the mutated `value`s.
Because the DP mutates values to the optimal allocation, the standard loop reproduces the DP's
optimum by construction: it recomputes `base + extra − lost` and the counters over exactly those
faces, applies its own `clamp`, and reports it. **The DP's only job is to choose each die's `k`;**
the b-1 net/counter/clamp/tier arithmetic is not touched or duplicated. This minimizes blast radius
and keeps M11a/b-1's counting logic sealed.

Consequence to preserve: whatever objective the DP maximizes must be *exactly* what
`evaluate_success` will then compute from the mutated values — clamped net successes, plus the
`positive_counter`/`negative_counter` sums. §4 pins this.

## 3. The four resolutions (brainstorm outcomes)

### R1 — Objective optimizes the *visible* (clamped) outcome

§8's objective is "maximize net successes, tie-break net counters." §7 **defines** net successes as
`clamp(base + Σextra − Σlost, 0)` (floor 0 unless `allow_negative`). So the objective's primary key
is the **clamped** value, matching what `evaluate_success` reports and what the oracle reads off the
real outcome. This forks the DP in the all-failed region (best achievable raw net ≤ 0): there every
allocation clamps to net 0, ties on successes, and **counters alone decide** — see §4.

### R2 — Expertise adjusts from `value`, not `natural`

§8 literally writes `clamp(natural ± k)` but the result lands in `value`. For any die where
`value ≠ natural` (today: a Penetrated child, `value = natural − 1`; tomorrow: any value-adjusting
modifier), reading `natural` would (a) discard that adjustment and (b) make the `k = 0` value
function score `natural` instead of the die's real current contribution — corrupting the baseline
even when zero points are spent on the die. **Corrected: expertise adjusts from `value`:**
`f = clamp(value + step·k, min, max)`, so `v_i(0)` reproduces the die's current contribution
exactly. For the ~99% of dice where `value == natural` this is identical to §8's wording; it only
bites value≠natural dice.

### R3 — Deterministic allocation tie-break: lowest-index-first

Distinct point distributions can achieve the identical `(clamped_net, net_counters)` while touching
different dice, so `DieRecord.expertise` (a player-visible audit trail) would differ — violating the
`evaluate`-determinism invariant. **Canonical rule:** process dice in pool order; in the DP's inner
`max over k`, on an objective tie prefer the **smaller `k`**; the first argmax encountered wins.
The differential oracle applies the *same* rule so `DP == oracle` including the exact allocation.

### R4 — `e<N>` notation: roll-level, applied only where meaningful, never a hard error

`e<N>` is a **roll-level** token (shared parser state, like `t<N>` — not a `DiceGroup` modifier),
setting `expertise = N`. It populates `SuccessConfig.expertise` **only** when the resolved mode is
`SuccessCount` with a success rule. When the resolved mode can't use it (Total-ambient), the token
is **parsed and silently discarded — never a `ParseError`** — so an identical string is portable
across systems (a Total-mode system just ignores the annotation). A **duplicate** `e<N>` in one
roll remains a genuine authoring error → `ParseError` (mirrors `DuplicateSuccessRule`); the lenience
is only for the inapplicable-mode case, not for contradictory input.

## 4. The DP (grounded, provable)

**Per-die value function.** For kept die `i` (current face `value` `v`, bounds `[m, M]`, direction
`dir`, config `cfg`), for `k` in `0..=E`:

```
step = if dir == HighWins { +1 } else { -1 }        // "toward better"
f    = clamp(v + step*k, m, M)                        // R2: base on value
base = cfg.success.comp.test(f, cfg.success.target) ? 1 : 0
dc   = crit::score_die(dir, f, cfg)                   // reuses b-1 crit scoring verbatim
net_i(k)     = base + dc.extra_successes - dc.lost
counter_i(k) = dc.positive_counter - dc.negative_counter
v_i(k)       = (net_i(k), counter_i(k))               // a pair
```

`v_i` is *typically* nondecreasing in both components as `k` rises (moving toward better only starts
crit-successes / stops crit-fails), but the DP **does not rely** on monotonicity or concavity — it
enumerates every `k`, so perverse configs (e.g. a negative `positive_counter`) are handled
correctly. This is why §8 rejects the `O(N·E)` convex-hull variant.

**Lexicographic DP (the raw-net pass).** Objective key `(Σ net_i, Σ counter_i)` compared
lexicographically:

```
dp[0..=E] : best (net_sum, counter_sum) over dice processed so far, using ≤ j points
init dp[j] = (0, 0)
for each kept die i:
    dp'[j] = lexmax over k in 0..=j of  dp[j-k] ⊕ v_i(k)      // ⊕ adds both components
answer_raw = dp[E]; backtrack the argmax k per die (R3 tie-break) → allocation A_raw, R* = its net
```

`O(N·E²)`. `E` is single digits, `N` dozens → trivially cheap.

**Applying R1 (clamp).** The clamp is non-additive, so it is applied *around* the DP, not inside it:

1. `allow_negative == true` → objective is unclamped; **use `A_raw`.**
2. `allow_negative == false` **and** `R* ≥ 1` → clamp is identity at and above the optimum, so the
   raw-net lexmax equals the clamped lexmax; **use `A_raw`.**
3. `allow_negative == false` **and** `R* ≤ 0` → *every* allocation clamps to net 0, so successes
   tie and the objective degenerates to **pure counter-maximization**. Run the DP a second time with
   the first key dropped (objective = `Σ counter_i`, single key, same R3 tie-break) → `A_counter`;
   **use `A_counter`.**

Still `O(N·E²)` overall (≤ two passes). The oracle validates this end-to-end (§6), so any flaw in
the three-case split is caught empirically, not just by inspection.

Once the winning allocation is chosen, `eval::expertise` writes each die's `f` into `value` and `k`
into `expertise`; `evaluate_success` then produces the reported outcome (§2).

## 5. Struct & pipeline changes

- `spec.rs`: add `expertise: u32` to `SuccessConfig` (`0` = off). Update all construction sites
  (tests) mechanically.
- `outcome.rs`: add `expertise: i32` to `DieRecord` (default `0`; carries points spent). Update
  `resolve_group`/`push_extra` and test constructors.
- `eval/expertise.rs` (new): the DP, the value function, backtracking, and the two-pass clamp
  handling. Pure; no I/O.
- `eval/success.rs`: call `eval::expertise` on the pooled kept dice when `cfg.expertise > 0`, before
  the counting loop. No change to the counting/clamp/tier logic itself.
- `notation/{lexer,parser}`: lex `e`/`E` + integer; collect roll-level `expertise` in shared parser
  state; lower per R4.
- **b-2 operates on all kept dice** — every `DieKind` is `Numeric` until b-3. The ordered-numeric
  exclusion guard for `Faces` dice is a b-3 addition, noted here as a forward pointer.

## 6. Differential oracle (mandatory) + testing

**Oracle.** A brute-force **exhaustive** allocator: enumerate every distribution of `E` points over
`N` kept dice via stars-and-bars (feasible only because `E`, `N` are tiny in tests), score each by
the **same R1 clamped-lex objective with the R3 tie-break**, take the max. Property test:
`DP == oracle` (both the objective value **and** the exact per-die allocation) across a large random
corpus varying `[min,max]`, success target, crit configs (**including perverse counter signs**),
`direction`, `allow_negative`, `E ∈ 0..~6`, small `N`.

**Additional tests:**

- **Bounds:** result net ≥ greedy baseline; ≤ theoretical max; `expertise = 0` ⇒ outcome identical
  to no-expertise (pins `v_i(0)`).
- **R1 fork (all-failed):** `allow_negative = false`, a pool that cannot reach 1 net success ⇒ the
  counter-max fallback picks the max-counter allocation, not the raw-net-lex one.
- **R2 (penetrate):** a die with `value = natural − 1` ⇒ expertise adjusts from `value`;
  `v_i(0)` preserves `value` (no free refund at 0 points).
- **R3 (determinism):** same `(spec, raws)` ⇒ byte-identical allocation.
- **Direction mirror:** flip `direction` + mirror every face (`f → min+max−f`) ⇒ net/counters/tier
  invariant.
- **Composition:** expertise pushing net across a `required_successes` tier boundary
  (expertise ↔ crit ↔ tier compose without special-casing).
- **Notation:** `4d6e3` under SuccessCount+success ⇒ `expertise = 3`; under Total-ambient ⇒ parsed,
  discarded, no error; duplicate `e` ⇒ `ParseError`.

**Buddy-check** on the DP task is pre-authorized (standing §8/§4.1 directive — the single
highest-risk piece of the engine).

## 7. Deferred (consistent with existing precedent)

- **Bounding `expertise: u32`** against DoS (`O(N·E²)` with a huge `E`) belongs at the **M11d**
  untrusted-transport boundary, alongside the not-yet-built per-roll dice-count cap. b-2 is a pure
  library and stays seed-/cap-agnostic. Log to `docs/TODO.md` next to the dice-count cap item.
- ts-rs bindings / wire frames → M11d.
- Labeled + custom-face (`Faces`) dice, and the ordered-numeric expertise exclusion guard → M11b-3.

## 8. Codebase-skill gate

On merge, update `shadowcat-codebase-dice` (new `eval::expertise` stage + value-mutating-pre-pass
seam + `SuccessConfig.expertise`/`DieRecord.expertise` + the R1 clamped-objective invariant +
`e<N>` notation) and have `shadowcat-spec-reviewer` confirm the diff against source — same tier as
the doc-sync gate.
