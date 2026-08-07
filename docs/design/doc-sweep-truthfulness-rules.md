# Doc-sweep truthfulness rules

**Hand this file to every doc-sweep implementer and reviewer** (by path — do not paste the rules
into a brief, or they drift between dispatches).

## RULE 0 — GOVERNING: never work around a rule; follow its INTENT, and ask when unsure

User directive, verbatim, binding on every agent in this campaign:

> "we do not try to work around rules, ever. we accept the intent of the rule and follow it.
>  if we are unsure of the intent, ask the user."

This outranks every rule below, because these are the rules that get worked around. Reworking text,
code, or scope until a rule no longer applies is never acceptable — not when the result is
technically true, not when `lint:docs` reports 0. **Every rule in this file was written after a
specific defect, so its letter encodes one observed instance while its intent covers the whole
class.** Satisfying the letter against the intent reproduces the original defect in a new shape, and
does so *while reporting clean*, because the check is what got satisfied.

The concrete case that produced this rule: Rule 15 banned file-name citations, so an agent that hit
a claim no symbol could cite rewrote `mirrors channels.ts's shape` → `mirrors the chat view model's
shape`. The filename is gone and the letter is satisfied; the citation now points **nowhere**, which
is worse than the stale pointer it replaced and is precisely what Rule 15 exists to prevent. Rule
15's "DO NOT VAGUE THE PROSE" subsection is one instance of this failure; the principle is general.

Three shapes, all of which present as compliance:

- **Rewording until the rule stops applying** — the case above.
- **Satisfying the check while defeating its purpose** — an empty `/** */` that clears the docs
  gate, `@returns The result.`, a test that asserts nothing, a cast that hides a real mismatch.
- **Reading a rule narrowly to shrink the work** — also a descope, separately forbidden.

**Never cut scope or coverage to resolve a hard case.** A second standing user directive — *"never
ever make descoping decisions without consulting me first"* — binds the same way, and surfacing a cut
after making it does not count as consulting. Exempting a category from a rule, narrowing a file's
scope, skipping a walk because someone checked part of it, or downgrading a full re-derivation to a
spot-check are all cuts. If you believe something is out of scope, that is a **finding for your
report**, not a decision you get to make.

**It arrives as a tie-break, not as a shortcut.** Two passes disagree, or two sections of one file
follow different conventions, and picking one feels like adjudication rather than a scope decision.
It isn't: *"which of these is in scope"* **is** scope. The dispatcher made exactly this error twice
in one hour of the Rule 15 pass — exempting a whole category to settle a disagreement between two
forks, and reducing a required walk to a spot-check — and both felt like unblocking rather than
cutting. Unblock by holding: tell the agent to leave the case untouched and list it, then ask.

**Local consistency is not authority.** A convention observed inside one file is evidence about that
file, not about this campaign. When two conventions collide, the rule's own text decides — not
whichever one is nearer to the line being edited. Two sections can disagree because one is already
wrong.

**How to apply.** Difficulty satisfying a rule is the signal to ask what it is FOR, never to find the
minimum edit that clears it. If a rule's intent cannot be followed, or is genuinely ambiguous,
**surface it to the dispatcher, who takes it to the user** — never resolve it silently in the
artifact. An honestly reported "I could not comply" is worth more than a clean count: the dispatcher
can adjudicate an exception and cannot even see a reworded one. A rule that is genuinely wrong gets
raised and changed, not routed around. Carry this into every dispatch.

Every rule traces to a specific defect the `client/core` (620 warnings), `client/render` (339),
`client/shell`+`ui-kit`+`formula` (276), and `module-panels` (217) sweeps shipped and caught, ordered
by measured yield. Across all four, **every fix round was triggered by a doc sentence asserting
something FALSE, or by a citation that did not support it** — never by a missing comment. Plan review
effort accordingly: the risk in a documentation sweep is not absent prose, it is confident prose that
a future agent will trust and build on.

**Rules 1–12 govern what a sentence claims; Rules 13–15 govern whether its citation can be checked at
all.** Sweep 10 is why that distinction earns its own rule: three consecutive tasks wrote claims
tables full of true claims and wrong pointers, and no amount of instruction fixed it. **Rule 15 is the
governing citation rule for anything persisted, and it overrides Rule 13 wherever they conflict** —
Rule 13 now binds only the ephemeral review artifacts named in its own scope line.

**The `client/render` sweep found more defects outside the docs than in them** — a real rendering
bug, two defects in the docs gate itself, a runtime divergence between sibling files, and drift in
a codebase skill — every one surfaced by trying to *verify* a claim rather than write one. That is
the strongest available argument for these rules: a documented property is a falsifiable one, and
nobody tests "does it draw every hex" until a comment claims it does.

**A cautionary case, because it is the most repeated failure here.** One guard's justification was
written three times. Attempt 1: "an oversized magnitude reaches the client as `Infinity`." Attempt
2, correcting it: "it arrives as `null`, because the server's round-trip nulls non-finite floats."
Both false — `serde_json`'s lexer rejects an overflowing literal at tokenization, so neither value
ever exists. Attempt 2 cited a real crate, a real function, and a real behavior of that function,
and was still wrong because nobody checked whether that function is *reached* on the path. The
guard was correct all three times; only the story about *why* kept being invented, and each version
was more detailed and equally unverified. The fix was Rule 4: delete the mechanism rather than
attempt a third. **A plausible causal story is the single easiest thing to get wrong, and detail is
not evidence.**

**The `client/shell` sweep's own contribution is Rule 12, and it is about this document's blind spot.**
Rules 1–11 govern prose written *from code*. Sweep 9's four fix rounds were all triggered instead by
prose written *from another agent's finding* — a reviewer's, an implementer's, or an earlier summary
of my own — where the source was true and the restatement was not. That path has no claims-table row
and feels like reporting rather than claiming, which is exactly why it went unchecked four times.

Scorecards referenced below are cumulative over all three sweeps and worth continuing to track.

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
reader. Severity does not help — every doc check is now an error that fails CI, and an error about
presence is still only about presence:

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
because the gate, the census, and the task list are all blind here by construction: a file already at
zero is invisible to a count, and a false claim inside it costs nothing to leave.

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

## RULE 12 — a relayed finding is an uncited claim, and it fails by getting WIDER

Sweep 9's four fix rounds were all triggered by a claim that was true at its source and false as
restated. Nobody fabricated anything; each relay dropped a qualifier:

| Source claim (TRUE) | Relayed as (FALSE) |
|---|---|
| no test exercises the CANCELLATION scenario (`mod` preceded by `+`, not `-`) | "no existing test covers a negative substitution" — `template.test.ts:32-35` covers exactly that |
| `TokenSelection.set` re-triggers when the set starts non-empty | "`TokenSelection.set` always re-triggers" — `SvelteSet.clear()` early-returns on empty, so empty→empty is a genuine no-op |
| the mock's `addLayerFilter` dispose is STRICTER (removes by identity) | "the mock is more permissive" — inverted |
| `RenderEngine` exposes viewport control | "`RenderEngine.resize`" — no such method; it is `setViewport` |

The relay is where the qualifier dies, because a qualifier reads like hedging when you are compressing
someone else's finding into one line. The generalized version is always cleaner prose and always a
weaker claim's replacement by a stronger one.

**How to apply.** Treat a finding from a reviewer, an implementer's report, or your own earlier summary
exactly like prose you are about to ship: it needs its own `file:line`, verified now. This costs one
`grep` — three of the four above were disproven by opening the cited file. Watch specifically for
absolutes appearing during compression ("always", "never", "no test", "only") that were not in the
source: RULE 5 says absolutes concentrate the errors, and a relay is where they get INTRODUCED. When
the source scoped a claim to a specific condition, the restatement keeps that condition or drops the
claim.

## RULE 13 — in EPHEMERAL artifacts only: cite PATH-QUALIFIED locations, and verify the table mechanically

**Scope: claims tables, implementer reports, review packages, and dispatch prose — artifacts written
to `.superpowers/`, read within the hour, and never merged.** For anything persisted into the repo
(doc comments, `OPEN_BUGS.md`, `POST_WORK_FINDINGS.md`, `TODO.md`, `PLAN.md`, `CLOSED_BUGS.md`, and
this file), **Rule 15 governs instead and forbids what this rule requires.** A line number is
acceptable here for exactly the reason it is banned there: these artifacts are consumed before the
tree moves under them.


Sweep 10's contribution, and the only rule here that instruction alone could not fix. Three
consecutive tasks shipped claims tables whose `file:line` citations pointed at the wrong lines —
7 of 11 rows, then 8 of 23 — **after two dispatches explicitly said "generate the table last,
against the committed file."** Every underlying claim was true; only the pointers were wrong.

The mechanism, confirmed by the implementer that produced it: anchor line numbers are captured with
a grep partway through the work, editing continues, and the table is then written by counting
offsets from that stale list. It feels like recall, not invention, which is why care does not
prevent it.

Two requirements follow, and the first matters more than the second:

**1. Path-qualify every citation.** Write `src/modules/panels/src/controller.svelte.ts:32`, never
`controller.svelte.ts:32`. This repo has two `controller.svelte.ts` and 26 `index.ts`; line 19 of one
is a `@param` line and of the other `export interface ToolContext {`. A bare basename is not merely
error-prone — it is **unverifiable by construction**, by a human or a tool, so drift in it is
undetectable rather than merely unnoticed.

**2. Verify the table mechanically, after the last source edit.** Print every citation beside the
line it actually lands on and read them. A citation landing inside a JSDoc block is correct for a row
whose claim is about documentation TEXT and drift for a row claiming to cite CODE — only the row's
own wording decides which, so no checker can classify it for you.

Two calibration failures are worth repeating, because both made a checker worse than none:

- Resolving an ambiguous basename by accepting the citation if ANY candidate file held code at that
  line **passed the exact rows two reviewers had already caught.** A verification step that guesses
  converts "unchecked" into "checked", which is strictly worse than leaving it unchecked.
- Flagging every comment-landing citation as suspect produced 15 false positives out of 51 on a table
  that was correct. A check that cries wolf on good work gets ignored, and then catches nothing.

The claims table's whole function is letting a reviewer confirm a claim by opening one location.
Wrong pointers convert it into decoration that still reads as diligence — worse than an absent table,
which at least advertises that nothing was verified.

## RULE 14 — a green `lint:docs` scopes the eye to what it counts, and Rule 7's re-scan inherits that scope

The gate counts `/** */` blocks with the right tags present, and it has no opinion at all on a
standalone `//` comment anywhere. `pnpm lint:docs` also declines the bare
`ArrowFunctionExpression`/`FunctionExpression` selectors, because they would fire on every inline
callback; `pnpm lint:props` covers the narrow named-binding cases those miss — an exported or
module-level `const` bound to an arrow or function expression — but a `const` holding a plain VALUE
is gated by neither. Every prior sweep's Rule 7 re-scan silently inherited the gate's blind spots by
re-checking only `/** */` blocks, which is not what Rule 7 promised.

Evidence, all from Sweep 11:
- Two of the sweep's best findings were false comments on `const` declarations, which `require-jsdoc`
  never gates — `ROUTE_PREVIEW_DEBOUNCE_MS`'s "at most one request per window" (falsified by its own
  trailing fire) and `DRAG_THROTTLE_MS`'s entire pre-D9 vocabulary.
- Task 2's spec review found inline `//` comments un-enumerated campaign-wide — no sweep, including the
  nine already merged, had ever systematically walked them.
- Task 4's spec review found three comments never inspected, one of them stale — and diagnosed the cause
  as a **miscounted enumeration**, not a short list: the re-scanner believed it had covered the file.

**How to apply.** A green `lint:docs` for a file or package is evidence its JSDoc *tags* are present, not
that its comments — of any kind — are true. When Rule 7 requires a re-scan, that re-scan's surface is
every comment in the touched files (`/** */` blocks AND standalone `//` lines), not the subset the gate
happens to count. State the enumeration as a number, not an impression, the same way Rule 11 asks for an
explicit member list.

## RULE 15 — persisted prose cites SYMBOLS, never file names or line numbers

**The rule.** In any prose that gets committed — doc comments in `.ts`/`.rs`/`.svelte`, and the live
tracking docs (`OPEN_BUGS.md`, `POST_WORK_FINDINGS.md`, `TODO.md`, `PLAN.md`, `CLOSED_BUGS.md`, this
file) — cite the **type name and member**, never a path and never a line number.

```
BAD    see src/server/src/ws/conn.rs:1313-1329
BAD    see conn.rs (the scene_subscribe arm)
BAD    `revs` is bumped only by onAssetChanged — see assets.ts:41-45
GOOD   see `egress_loop`'s `SceneSubscribe` arm
GOOD   `AssetResolver.revs` is bumped only by `AssetResolver.onAssetChanged`
```

**Why this rule outranks Rule 13's path-qualification.** Rule 13 correctly diagnosed that bare
basenames are unverifiable, and prescribed path-qualified `file:line`. That prescription is
load-bearing for an artifact read within the hour and fatal for one that ships. A line number is
invalidated by *any* insertion above it, in *any* commit, by *anyone* — including the documentation
sweep itself, whose entire product is inserting comment blocks above existing code. The campaign has
been generating its own citation rot as a by-product of doing its job.

Measured, in this sweep alone: **eight citations in `OPEN_BUGS.md` went stale**, four of them from a
single five-line commit — and that commit was mine, which is why it was also misattributed to an
implementer as a Rule 13 violation before being withdrawn. No gate catches this. No linter parses
Markdown prose for pointers, and nothing at all checks a `file:line` written inside a `//` comment.
The rot is silent, it accumulates, and every instance aims a future reader at whatever code has since
drifted into those coordinates — which is strictly worse than no citation, because it still reads as
diligence. **A symbol name has none of this failure mode: it survives every insertion, every reflow,
and every move between files. It breaks only on rename or deletion — precisely the edit where a grep
for the old name finds it.**

**Disambiguation without paths.** Rule 13's motivating problem was real: this repo has two
`controller.svelte.ts` and 26 `index.ts`. Symbols solve it better than paths did. Qualify with the
**owner**, not the location — `AssetResolver.url`, `egress_loop`'s `SceneSubscribe` arm,
`EngineAdapter.dispose`. For a bare function with a common name, name the module that exports it
(`chat/mod.rs`'s `broadcast` → `chat::broadcast`). If a symbol is genuinely ambiguous after
owner-qualification, that ambiguity is a naming defect worth reporting, not a reason to reach for a
path.

**Do not smuggle the path back in.** "the `AssetChanged` handler in `assets.ts`" is this rule's
violation wearing prose clothes. Name the symbol and stop. Cross-document references drop the
extension too: "see OPEN_BUGS, the AssetChanged entry", not "see `docs/OPEN_BUGS.md:143-149`".

**Generated files are edited at their source.** `src/types/generated/engine/*.ts` is ts-rs output
whose doc comments are verbatim copies of the Rust `///` blocks in `src/server/src/data/engine/`.
Fix the Rust and regenerate; a citation "fixed" in the generated file is reverted by the next build.

**Files that have no symbols are named by path.** Config and build files — `eslint.config.js`,
`eslint.props.config.js`, `package.json`, `.github/workflows/ci.yml`, `tsconfig.json` — export no
type or member to cite, so naming the file *is* naming the thing. Same for a filename used as a
**value** rather than a pointer (a default like `index.js`, a glob, a URL). This rule governs
citations of *code behavior*, and neither of these is one.

**This document's own examples are exempt.** The BAD/GOOD blocks above and in Rule 13 must keep
their `file:line` text — they are the specimens the rule is defined by. "Correcting" them deletes
the rule's meaning.

**Historical records are exempt, and quotations of them are too.** Dated plan and spec files under
`docs/superpowers/` are records of what was true when written — rewriting them asserts a present-tense
precision they never claimed. Likewise, where this file quotes a *past* citation as evidence of a past
defect (Rules 1 and 12), the quoted `file:line` stays: it is a record of what someone wrote, not a
pointer this document is asking you to follow.

### Measuring the surface — anchor on the INVARIANT, never the delimiter

The Rule 15 pass undercounted its own scope **eight times**, and once more *after* being committed and
reported clean. Every miss had one cause: the pattern encoded the punctuation **around** a citation
instead of the citation's invariant.

| Pattern anchored on | What it could not see |
|---|---|
| `` `file.ts` `` backtick-delimited | unbackticked prose — `see mock-server.ts` |
| character class `[A-Za-z0-9_.-]` (no `/`) | backticked **paths** — `` `ws/conn.rs` ``, `` `data/sqlite.rs` `` |
| `` ` `` followed immediately by `)` | `` (`a.rs`, `b`) `` — 17 real sites reported as 7 |
| `file.ts:NNN` | `data/sqlite.rs::delete_user`, `(data/sqlite.rs Phase 1)` |

The invariant is the **filename token** — `\S+\.(ts|rs|svelte|…)`. Backticks, parentheses, colons and
possessives are punctuation the author chose freely; anchoring a measurement on freely-chosen
punctuation guarantees an undercount that reports as precision.

**Measure the vocabulary, never the count:**

```
grep -rhoE '[A-Za-z0-9_/.-]+\.(ts|rs|svelte|mjs|scss)\b' <paths> | sort | uniq -c | sort -rn
```

`-o` emits the matched token rather than the line, so the output is *the list of distinct things that
exist in the tree* — a fact, not a confirmation of the hypothesis that built the pattern. Unexpected
forms appear as entries instead of being silently absent. **Deliberately over-match, then subtract by
review**: over-broad plus manual exclusion is falsifiable, narrow plus a count is not. False positives
are cheap (`sequenced.ts` is a Rust field access) and are the price of seeing `ws/conn.rs`.

**The same failure recurs in the FILTER and in the VERIFICATION, not just the pattern.** Two more
instances, both of which reported clean:

- **An exclusion applied to `path:line:content` matches the PATH.** Piping a sweep through
  `grep -vE '…\.test\.ts…'` to drop a carve-out silently dropped **every citation inside every test
  file in the repo** — likewise `config.ts`, `panels.scss`, `build.rs`. A carve-out written for
  individual lines was excluding whole files. Split the prefix off and filter the **content field
  only**.
- **A verification that can pass for the wrong reason verifies nothing.** A `sed` requiring backticks
  the source lacked matched nothing; the grep used to confirm it matched an unrelated import
  elsewhere in the same file, and the conversion was reported as landed. **Check the edited LINE**
  (`sed -n '<line>p'`), never the file.

**The unifying root of all of it:** each is a check that can succeed for a reason unrelated to what
it claims to check. Of any check here, ask not "did it pass?" but **"what else could make this
pass?"**

**Scope gaps are not pattern gaps.** `src/server/tests/`, `examples/` and `Cargo.toml` comments held
citations because no agent was ever assigned them — `examples/` still carried the original
`file:line` form plus bare `:NNN` after the whole campaign had reported complete. No pattern fixes
that. Enumerate containers — every directory, every file type — exhaustively, never from search hits.

Three consequences bind every task:

1. **State the first number as a floor, in writing** — "N under pattern P; unrecognised forms
   unmeasured", never "the surface is N".
2. **Close with a different method than the one that scoped it.** A residual sweep reusing the scoping
   pattern re-confirms the original blind spot and returns a clean zero.
3. **Ask implementers for their own count before revealing yours**, and treat any delta as a finding.
   An implementer inside the files sees forms the dispatcher's pattern cannot; stating the expected
   number first anchors them to it.

## Report contract

The implementer's report must carry a claims table (claim → verifying **symbol**, optionally plus a
`file:line` for the reviewer's convenience — the report is ephemeral, so Rule 13 governs it, but the
symbol column is mandatory because it is what the shipped prose must cite under Rule 15). Corrections
in a fix round carry the same burden as new claims. State explicitly what the pre-existing-prose re-scan
covered. Report a discovered divergence rather than smoothing it over — but bound its reachability.

## RULE 16 — code comments are durable commentary about THE CODE, and nothing else

User directive, iron-clad, in its governing form:

> as far as code is concerned, ephemeral documents, plans, dates, history, and tasks, do not exist.

and, stated earlier in its original form:

> task names, task ids, plans, dates, and repo documents should never be referenced in code comments
> because they are ephemeral. code comments should be durable commentary about the code and only the
> code.

**Read the governing form as an ontology, not a style preference.** The question is never "is this
reference useful?" — it is "does this thing exist, from inside the code?" A plan, a task id, a
review round, a date, a previous version of this function: none of them are visible from the code,
so a comment may not speak of them at all. This is why the rule admits no case-by-case exceptions:
every exception argues from usefulness, and usefulness was never the test.

RULE 15 said *how* to cite; this says *what a code comment may talk about at all*. It is the
stronger constraint and it wins wherever the two touch.

**Banned in `.ts` / `.rs` / `.svelte` comments — every form:**

| Banned | Example found in this repo |
|---|---|
| Milestone / task ids, in any form | `Kept minimal for M8c-1`, `M13-1 T1/T3`, and the bare `no engine consumer in M8` |
| Phase, workstream and numbered-invariant ids | `post-D9`, `W1's headerless stage group`, `break I4` |
| Repo document pointers | `` see `docs/TODO.md` ``, `` (`docs/OPEN_BUGS.md`, the AssetChanged entry) ``, `ARCHITECTURE §2 invariant 4`, bare `invariant 6` |
| Dated plan/spec files | `docs/superpowers/specs/2026-07-13-m11d-2-dice-chat-wire-design.md §7` |
| Unnamed spec references | `per spec §3.2`, `this mirrors the spec literally`, `the spec'd default` |
| Sweep / campaign / round / review markers | `sweep 13`, `fix round 1`, `buddy-check finding 4` |
| Dates stamped on a comment | `(2026-07-13)`, `as of August 2026` |
| History narration | `previously an Array`, `formerly client-owned`, `before the fix` |
| Process markers | `POST_WORK: replace with …` |

**Local numbering is not an exception.** An `I4` whose definition lives in a sibling comment of the
same subsystem still forces the reader to resolve a number that no compiler, test or tool binds to
anything. Ruled in scope by the user. The conversion states the invariant where it is load-bearing —
tersely, and without pasting the same paragraph at five sites, which RULE 3 forbids.

**The rule extends to code-facing string literals.** An `assert!` message and a test name are read
by a developer at failure time exactly as a comment is, and go stale the same undetectable way.
Ruled in scope by the user. A string that is *program data* — a fixture's world name, a document key
— names something inside the program and is untouched.

**Why these are one class, not five.** Each names something *outside the code* whose identity is
assigned by a process: a milestone gets renumbered, a doc section gets renumbered, a bug entry moves
from `OPEN_BUGS` to `CLOSED_BUGS`, a spec is superseded, a sweep ends. The comment then points at
nothing — and unlike a stale claim about code, **no reader and no tool can tell it went stale**,
because the referent's disappearance is invisible from the code. This is RULE 13's
"unverifiable by construction" defect, one level out.

**A milestone id feels different and is not.** `M13-0` reads like a fact about when something was
built. It is a fact about a plan, and the plan is finished — so the id survives as a token whose
meaning now lives only in a document the reader does not have.

**The conversion is always the same move: state the CONSTRAINT, drop the POINTER.** The pointer was
never the information; it was a shortcut to information that belongs inline.

```
BAD    // Kept minimal for M8c-1 (background + grid + camera); M8d generalizes to a node model.
GOOD   // Minimal by construction: background + grid + camera only. Adding a node kind requires
       // a matching `DisplayBackend` member and an implementation in every backend.

BAD    // KNOWN DEFECT (`docs/OPEN_BUGS.md`, the AssetChanged entry): `revs` is not bumped.
GOOD   // `revs` is NOT bumped by `onAssetChanged`, so a re-uploaded asset keeps its cached URL
       // until the next full resync. Callers needing freshness must re-fetch explicitly.

BAD    // Richer mismatch detection is deferred to module management (see TODO.md).
GOOD   // TODO: Detect version mismatches beyond exact-equality.
```

Note the third: `TODO:` is a *code* marker and stays. What gets deleted is the pointer to where the
deferral is tracked, per this repo's commenting rules ("No Process Meta").

**Where the tracking reference belongs instead.** The backlog entry, bug record or plan cites the
SYMBOL (Rule 15) and points inward at the code. The dependency runs doc → code, never code → doc,
so renaming or closing the doc entry cannot rot a comment. A defect worth warning a reader about is
worth stating as a present constraint in the comment itself.

**No grandfathering — the rule applies retroactively.** User directive: *"this is retroactively
applied. do not grandfather in existing cases."* `scripts/check-comment-refs.mjs` therefore carries
no baseline and no allowlist: every hit fails, legacy included. The reason a baseline is not
available here is structural — a grandfathered site is indistinguishable from a new one to every
future reader, so exempting the backlog would preserve exactly the defect the rule removes.

**What the detector can and cannot see.** The patterns cover the id-shaped and pointer-shaped forms
at high precision. History narration is only partly detectable: `previously`/`formerly` match, but
`no longer` overwhelmingly describes *runtime data* ("an id that no longer names a scene is
ignored") rather than the code's past, and flagging it would train writers to dodge the word instead
of dropping the narration. **A green detector is therefore not a satisfied rule** — history
narration is a review obligation. Reword to evade a pattern while still speaking of something
outside the code and you have violated RULE 0, not fixed anything.

**Scope.** Code comments and code-facing strings. Markdown documents, skills and `.superpowers/`
artifacts may
reference other documents by path + section anchor — prose has no symbols, and those artifacts are
read as documents. Do not carry this rule's prohibition into them, and do not carry their allowance
back into code.
