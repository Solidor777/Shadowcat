# Doc-sweep truthfulness rules

**Hand this file to every doc-sweep implementer and reviewer** (by path — do not paste the rules
into a brief, or they drift between dispatches).

Every rule traces to a specific defect the `client/core` sweep (620 warnings) shipped and caught,
ordered by measured yield. That sweep needed **eight fix rounds across six tasks, and every single
one was triggered by a doc sentence asserting something FALSE** — never by a missing comment. Plan
review effort accordingly: the risk in a documentation sweep is not absent prose, it is confident
prose that a future agent will trust and build on.

Scorecards referenced below are cumulative over that sweep and worth continuing to track.

## RULE 1 — a citation must support THE CLAIM AS WORDED, not an adjacent fact

Highest yield. Caught a defect in three consecutive tasks, every one of which had a citation attached
and would have passed a "does it have a row?" check:

- **T2:** "The **sole caller**, `computePull`" — cited `templates.ts:87-90`, which verifies the fallback
  *expression* and says nothing about *exclusivity*. (`merge3` also recurses via `merge3Embedded`.)
- **T4:** "sessions are revoked **inside the same transaction**" — cited `routes.rs:438-449`, the very
  lines showing `repo.delete_user()` committing its own tx and THEN `rooms.evict_user()` running after it.
- **T4:** "writable on **ANY doc_type**" — cited `permission.rs:195-196`, accurate in isolation, but an
  earlier gate (`sqlite.rs:2187-2190`) means that function is never reached for `message` docs.

Audit the proposition, not the presence of a link. A citation that is true *in isolation* but not
*operative* is the subtlest form of this.

## RULE 2 — every non-obvious claim needs a claims-table row; uncited is where the false ones live

Measured across the sweep: claims WITH a row ran ~7/8 accurate. Every claim that turned out **false**
was one WITHOUT a row — **5/5**. Non-obvious = a default, a unit, a wire field/frame name, a timeout,
an ordering or atomicity guarantee, an authz statement, or any "the server does X" / "mirrors Y" /
"the client sends Y" sentence.

**Refinement:** uncited does not *imply* false — Task 6's hunt found two uncited claims, one false and
one true. The rule is that uncited is *where the false ones live*. The table's real function is to
force the author to open the enforcing file; claims that skip that step are the ones that go wrong.

Run two genuinely separate passes: (a) audit the table, (b) hunt claims that never entered it.
**Pass (b) is where every real defect has been found, and it is the pass reviewers skimp on.** On
Task 5 a reviewer returned a perfect 19/19 table audit AND "0 uncited claims" — the second number was
wrong, and the other reviewer found the defect by reading prose. A table audit structurally cannot
catch a claim absent from the table. Ask reviewers to state how hard they actually ran pass (b).

## RULE 3 — when a bound rests on several legs, keep the decisive one and CUT the rest

The through-line of T4. One true conclusion ("`isGm: true` has no production caller today", verifiable
by a single grep) was propped on three legs. Leg 1 was false in the restrictive direction ("`gm_role` is
set in exactly one place"), leg 2 was unsupported ("the sheet/panel callers never touch a `gm_role`-capped
document"), leg 3 was false in the permissive direction ("writable on ANY doc_type"). **Three fix rounds
went entirely to supporting claims the conclusion never needed.**

Extra legs read as thoroughness. Each one is a fresh falsifiable claim.

## RULE 4 — prefer DELETION over recomposition, and delete the WHOLE overclaim

Each recomposition is a fresh chance to assert something unverified; this sweep produced four wrong
sentences that way.

**And a deletion must cover the whole overclaim, not just the phrasing that was tested.** T4's controller
disproved one sentence, deleted exactly that sentence, and shipped the same substance stated differently
one line earlier. After deleting, re-read the WHOLE paragraph asking: "does anything still assert what I
just removed?"

**Beware the mirror-image overcorrection.** Fixing a too-restrictive claim by asserting the opposite
produced a claim that was false in the new direction (leg 1 → leg 3 above).

## RULE 5 — absolutes concentrate the errors

never / always / only / no / all / any / sole / solely / exactly one / unconditional / at all.

Enumerate the reachable cases behind each before writing it. Prefer narrowing an existing accurate
sentence over composing a new absolute. Time-scope where honest ("no production caller **today**" beat
"no production caller at all").

## RULE 6 — "mirrors X exactly" requires a condition-by-condition diff

T3's implementer claimed exact client/server ownership parity while citing the very lines containing the
counter-evidence (the server fail-closes on a scope mismatch; the client has no scope check). Misreading
toward the claim you want is the failure mode. Open both sides and line them up.

## RULE 7 — pre-existing prose is in scope, with the same skepticism as new text

The initial "no stale docs found" verdict has been wrong **three times**. T4's uncited false claim was
pre-existing prose, surfaced only because the re-scan was mandatory. "None found" is acceptable only
accompanied by a list of what was actually checked.

## RULE 8 — a true statement pinned to an unreachable code path still misleads

T4's implementer correctly found and documented a real client/server divergence (good — that is Rule 6
working), but attached it to `canWritePath`'s `isGm` branch, which has no production caller. A reader
would "fix" that function and change nothing, because the live bypass was one layer up in the caller.
When documenting a divergence, verify WHICH call path actually reaches it.

## RULE 9 — a green `lint:docs` does not mean the docs are well-formed

The gate measures tag **presence**, never placement, duplication, or whether the right prose reaches a
reader. It cannot catch these, and neither can the `warn`→`error` ratchet:

- **Appending a second JSDoc block instead of merging into the existing one.** `jsdoc/require-*`,
  TypeDoc, and tsserver hover all resolve to the **nearest preceding** block. A new tag block appended
  below a richer existing one satisfies the linter while **orphaning** the older block — its content
  never reaches the docs site or IDE hover. Task 6 did exactly this to `transport.ts`, orphaning the
  `settled`-flag rationale and the pre-open close/error double-schedule invariant, the most load-bearing
  prose in the file. `lint:docs` reported 0. Correct shape: remove the old closing `*/`, insert the new
  tags, re-close the one block.
- **A tag that exists but says nothing** (`@returns The result.`) passes `require-returns` identically
  to a real one.

So: when a task reports "N → 0", that is evidence the tags exist, not that the documentation is good.
Review the placement and the substance separately from the count.

## RULE 10 — a file at 0 warnings is not a file with correct docs, and the plan is blind to it

**A sweep plan's task list is built from the warning census, so every already-documented file is
invisible to it.** Nothing in the process forces anyone to open those files — which makes them the
best hiding place in the package for a stale or false claim.

Sweep 8 Task 1 hit this immediately. `token-animator.ts` documented `@param serverNow` as "defaults to
`Date.now`"; `Date.now` appears nowhere in that file's code, and omitting the argument yields elapsed
`0` (no catch-up at all) — a materially different behavior, reachable through a public seam. The same
false sentence also sat in `types.ts`, on the *interface* the method implements. `types.ts` had **zero
warnings**, so no task in the plan touched it: the claim would have survived the entire sweep, and the
newly-written implementation doc would have been left contradicting its own interface.

**How to apply.** When a sweep documents a symbol, grep the package (and its `types.ts`/interface
files) for the same claim, and check the interface the symbol implements. Better: give every sweep
plan an explicit step — "verify claims in already-clean files this sweep's subject matter touches" —
because the ratchet, the census, and the task list are all blind here by construction.

## RULE 11 — when two findings share a shape, stop hunting instances and audit that dimension

Two defects with the same *form* are evidence of a blind spot, not a coincidence. Switch from
opportunistic hunting to a systematic pass over the dimension they share, enumerating every member of
the relevant surface.

Sweep 8 Task 2 documented a real/mock backend pair. The implementer found 7 divergences; a code review
found an 8th (`addLayerFilter` dispose removes by value vs. by identity); a spec review found a 9th
(`startTicker` accumulates vs. replaces). Both new ones were **repeat-call semantics** — what happens
when a method is invoked twice — which no doc addressed for any method. Reframing that as a dimension
and walking all 21 methods with three questions produced **two more** (a 10th and 11th) that three prior
passes over the same two files had missed:

1. What does calling this **twice** do — accumulate, replace, no-op, or throw?
2. Do the two implementations **agree** on that answer?
3. Is the answer **documented**?

The 10th was the sharpest in the set: the real backend's `destroy()` throws on a second call (Pixi nulls
`stage`/`renderer`), while the mock's is silently idempotent — so a double-destroy test passes green and
production crashes. Ad-hoc reading had walked past it three times.

**How to apply.** Name the dimension explicitly, work from the *real* member list (read the interface,
don't reuse the list in the brief — the guessed list in this case omitted the method that yielded the
11th), bound each finding's reachability so it can ship as a contract caveat rather than a bug report,
and **report the negative result too**: a pass that lists only hits is indistinguishable from a pass
that stopped early. The explicit "these 17 agree" list is what makes a completeness claim credible.

## Report contract

The implementer's report must carry a claims table (claim → verifying `file:line`), and corrections in a
fix round carry the same burden as new claims. State explicitly what the pre-existing-prose re-scan
covered. Report a discovered divergence rather than smoothing it over — but bound its reachability.
