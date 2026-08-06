# Docs Sweep 13 — property, type & full-coverage documentation pass

Successor to sweep 12. Sweep 12 drives `lint:docs` to 0 for FUNCTIONS; this plan closes the
properties, types and named-arrow gaps that gate leaves entirely unmeasured.

## Why this plan exists

User directives, 2026-08-05, verbatim:

> "complete the full documentation and documentation verification pass via sdd. as per before, every
> documentation pass gets buddy checked until no problems are found. **continue until every function
> and property is documented.** when done, merge to master, and watch CI."

> "**every type and setting should also be documented, along with every property therein.**"

Sweep 12 drives `lint:docs` to 0, but that gate does **not** enforce the directive. `rulesAt`
(`eslint.docs.config.js:13-32`) requires docs on `FunctionDeclaration`, `MethodDefinition` and
`ClassDeclaration` only. **Properties are entirely ungated**, and so are named arrow-function
exports. Sweep 12 finishing at 0 is therefore necessary but not sufficient.

## Measured scope

Measured 2026-08-05 with throwaway eslint configs that reuse the shipped `files`/`ignores` globs and
swap only `jsdoc/require-jsdoc`'s `contexts`. **Those probe files are git-ignored scratch and are not
branch content** — the `contexts` column below is the reproduction recipe; Task 1 lands them for
real in a new, separate property-gate config (not `eslint.docs.config.js`'s `rulesAt` — see Task 1
below for why), at which point these numbers become reproducible from the tree alone.

| Gap | Contexts | Sites | Probe |
|---|---|---|---|
| Properties | `PropertyDefinition`, `TSPropertySignature`, `TSMethodSignature`, `TSEnumMember` | **1222** | `eslint.probe3.config.js` |
| Named arrow / fn expressions | exported + module-level `VariableDeclarator > ArrowFunctionExpression\|FunctionExpression` | **6** | `eslint.probe2.config.js` |
| **Gate total** | | **1228** | |
| Type declarations | `TSInterfaceDeclaration`, `TSTypeAliasDeclaration`, `TSEnumDeclaration` | **102** | `pnpm lint:props`, group-isolated |
| **TS gate total** | | **1330** | |

The three figures above are now measured from the shipped `eslint.props.config.js` (Task 1), not
from the throwaway probes this plan was drafted against. The properties and arrow counts are
unchanged from the probe estimates; the type-declaration count is **102, not the 101 this table
carried at drafting**, and the TS total is correspondingly **1330, not 1329**. Both the Task 1
implementer and the dispatcher measured 102 independently — the implementer via a group-isolated
run of the shipped config, the dispatcher via a separate types-only probe that also confirmed real
`.svelte` type declarations exist (`VisualKindEditor`, `MessageCard`, `Entry`).

**The cause of the −1 is unrecoverable and is deliberately not guessed at.** The drafting probes
(`eslint.probe*.config.js`) were git-ignored scratch and no longer exist, so their globs cannot be
compared against the shipped config's. Do not reason from the paragraph this one replaces: it
claimed every probe's raw output ran exactly one high because of an unused-eslint-disable artifact
at `core/src/hooks.ts:25`, and that subtracting one from the 1324 combined probe therefore yielded
the type count. That derivation produces 101 and is contradicted by two direct measurements of the
tree. The shipped config carries the `reportUnusedDisableDirectives: false` block for that file
(`eslint.props.config.js:105-108`), so no such artifact is present in any number above.

### Type declarations ARE in scope — and the `z.infer` idiom must be handled, not dodged

User directive, second message the same day: *"every type and setting should also be documented,
along with every property therein."* That settles it — `TSInterfaceDeclaration`,
`TSTypeAliasDeclaration` and `TSEnumDeclaration` join the gate.

I had initially excluded them for a real reason, recorded here because the reason survives even
though the conclusion did not. The dominant idiom in `wire.ts` is

```ts
/** Mirrors `crate::chat::ActorOwnerRef` (chat message attribution). */
export const ActorOwnerRefSchema = z.discriminatedUnion("kind", [...]);   // :29-33
export type WireActorOwnerRef = z.infer<typeof ActorOwnerRefSchema>;      // :34  ← would be flagged
```

The inferred alias has no content the schema's own doc does not already carry, so the *naive* way to
satisfy the rule is to restate it — a second copy that then drifts, which is precisely the failure
truthfulness **Rule 3** exists to prevent.

**The resolution is a pointer, not a copy.** An inferred alias's doc says what the type IS and where
its meaning lives — `/** The inferred TS shape of `ActorOwnerRefSchema` above. */` — and stops.
That satisfies the directive and Rule 3 simultaneously. **Any brief covering a file with `z.infer`
aliases must carry this instruction explicitly**, because the path of least resistance for an
implementer facing 86 sites in `wire.ts` is to paste the schema's description onto the alias, and
that is the single highest-volume copy-drift risk in the whole campaign.

### The Rust half of "every type and setting"

The settings surface spans both languages: `world-settings`, `chat-settings`, `dice-settings`,
`vision-modes`, `light-gradation` and the registry docs each have a Rust engine struct
(`src/server/src/data/engine/registries.rs`) and a TS mirror. **eslint cannot see the Rust half at
all**, so the TS gate reaching 0 says nothing about it.

Measured 2026-08-05, and the measurement itself needs care:

- `cargo clippy -- -W missing_docs` reported **1** — misleading. `missing_docs` only fires on items
  public at the crate boundary, and `shadowcat` is a bin crate, so nearly everything is invisible to
  it. The single hit was `tests/assets.rs`'s crate-level doc.
- `cargo clippy -- -W clippy::missing_docs_in_private_items` reported **0** across the crate.

**Treat that 0 as UNPROVEN, not as a result.** Two things must be checked before any task relies on
it: (a) clippy genuinely re-ran rather than serving a cached result — a repeat invocation finished
in 0.94s having compiled nothing; (b) the lint actually fires, mutation-proved by adding an
undocumented private item and watching it report. A restriction lint that is silently not applied
looks exactly like full coverage. **Task 1 owns this proof.** A plausible outcome is that the Rust
side really is at zero — the crate's doc discipline is high and every function read during sweeps
1-12 carried a rich doc comment — but "plausible" is not the standard this campaign uses.

### Per-package distribution (properties gap)

`client/core` 492 · `client/render` 285 · `modules/panels` 162 · `client/ui-kit` 89 ·
`client/shell` 85 · `scene-tools` 49 · `actors` 43 · `formula` 38 · `chat` 29 · `chat-card` 10 ·
`entry` 9 · `sheet-actor` 6 · `stage` 6 · `sheet-item` 5 · `chat-composer` 4 · `assets` 3 ·
`sheet-fallback` 3 · `examples/system-minimal` 3 · `examples/module-initiative-tracker` 3 ·
`settings` 1. *(Counts from the combined probe; per-task briefs re-measure under probe 3 before
dispatch — a brief's scope table is verified against live lint, never carried forward.)*

Heaviest files: `core/wire.ts` 86 · `core/ws-client.ts` 71 · `render/engine.ts` 70 ·
`render/types.ts` 61 · `core/modules.ts` 42 · `render/backend.mock.ts` 36 ·
`render/pixi-backend.ts` 34 · `core/merge.ts` 33 · `core/actor.ts` 28 · `core/contributions.ts` 27.

## Task decomposition

Same loop as sweep 12 throughout: `shadowcat-coder` (effort medium) implements → dispatcher verifies
gates itself and audits citations → `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (effort
high) as the two-reviewer pair → fix rounds, five-round breaker → per-crate buddy-check convergence
before the ratchet.

**Task 1's config lives in a NEW, separate file, not `eslint.docs.config.js`.** That file's
ratcheted `.ts`/`.svelte` blocks now carry `files` globs byte-identical to their warn-tier
siblings (Sweep 12 Task 8), so flat config's later-block-wins-per-rule-key semantics mean the warn
tier is fully shadowed there — it cannot stage anything, because `rulesAt` is one function feeding
both tiers (`eslint.docs.config.js:10-12`) and a shadowed warn block never gets a chance to be
"the still-warn one" for a newly added rule. Adding a property context to that file's `rulesAt`
would land at `error`, repo-wide, immediately, with ~1,222 sites failing on day one. Task 1
therefore creates `eslint.props.config.js`: its own `rulesAt`-shaped severity function, its own
warn/ratcheted block pair (globs starting empty/narrow and widened per completed package, mirroring
how `eslint.docs.config.js` itself was built up sweep-by-sweep before Sweep 12), and its own
`lint:props` package-json script. It stays a fully independent ESLint invocation from
`eslint.docs.config.js` for the whole of this sweep, so neither can shadow or interfere with the
other. Task 15 merges both into `eslint.config.js` once `lint:props` also reaches 0.

**Three corrections from Task 1's implementation — this section's drafting assumptions were wrong:**

1. **A ratchet block cannot start "empty".** Flat config rejects `files: []` outright
   (`TypeError: Key "files": Expected value to be a non-empty array`). The ratcheted blocks start on
   a placeholder glob matching no real path. Tasks widening the ratchet REPLACE the placeholder;
   they never delete the key. And the ratchet glob must never be widened to equal the warn glob —
   that is precisely the shadowing this file exists to avoid. The endgame is Task 15's merge and
   this file's deletion, not a wide-glob steady state here.

2. **`require-jsdoc` alone gates only comment EXISTENCE.** The other five rules
   (`require-description`, `require-param`, `require-param-description`, `require-returns`,
   `require-example`) carry `contextDefaults: true` in eslint-plugin-jsdoc: with no explicit
   `contexts` option they silently fall back to `ArrowFunctionExpression`, `FunctionDeclaration`,
   `FunctionExpression`, `TSDeclareFunction` and NOTHING ELSE. As first written, an empty `/** */`
   satisfied the gate on 1324 of 1330 sites. Fixed in Task 1 fix round 1 by giving the content
   rules explicit contexts: `require-description` everywhere; `require-param`/
   `require-param-description`/`require-returns` on `TSMethodSignature` + the arrow selectors only;
   `require-example` deliberately NOT extended to properties/types (an `@example` per interface
   property is noise and would balloon `docs:check-examples`). **Any future rule added to either
   config must state which contexts it actually visits — a rule listed in `rulesAt` is not thereby
   enforced on the contexts in `require-jsdoc`'s list.**

3. **Scope boundary — USER-RATIFIED 2026-08-06, not a dispatcher judgment call.**
   `TSIndexSignature` IS in scope (declared type surface; 1 live site,
   `src/client/core/src/scene-docs.ts:522`). Object-literal properties (`ObjectExpression >
   Property`) are OUT — 3447 sites repo-wide. Do not add that context without an explicit
   instruction from the user.

   The first stated rationale — "all value positions rather than declared API surface" — is **too
   coarse and is wrong for one file.** `src/client/core/src/wire.ts` declares ZERO interfaces: the
   wire protocol's fields are Zod schema properties (`z.object({ kind: z.literal("actor"),
   actor_id: z.string() })`), which ARE object-literal properties and ARE declared API surface. 176
   of the 3447 live there. Only three files import zod (`wire.ts`, `manifest.ts`, `chat-docs.ts`),
   so scoping them in would be a bounded add, not the full 3447.

   **The rationale that actually holds is Rule 3 — no third copy.** Those fields' statement of
   record is the Rust server type; the client schema already points at it
   (`/** Mirrors `crate::chat::ActorOwnerRef` … */`). Per-field client docs would be a second copy
   of the server's semantics, in another language in another file, which the gate cannot detect
   drifting — it only checks a comment EXISTS, never that it is still true. The bulk of the 3447 is
   separately unfit for gating: the largest single contributor is the i18n catalog
   (`client/ui-kit/src/locales/en.ts`, 300 sites, entries like `"login.error": "Invalid username or
   password."`).

   **Process note, standing:** this exclusion was originally decided AND written into this plan
   before the user was asked. That was wrong — see the user directive of 2026-08-06: never make a
   descoping decision without consulting first; surfacing a cut after the fact is not consulting.
   Any future narrowing of this campaign's scope goes to the user BEFORE it reaches an artifact.

| # | Scope | Sites |
|---|---|---|
| 1 | **Config + measurement task. DONE.** Created `eslint.props.config.js` (see above) with a `rulesAt`-shaped severity function gating the property contexts, the 3 type-declaration contexts, `TSIndexSignature`, and the 4 arrow contexts, at `warn` repo-wide. Added a `lint:props` script. Every context mutation-proved individually. Rust side proved: `clippy::missing_docs_in_private_items` is already `deny`-enforced at crate root (`src/server/src/main.rs:15-16`), forced to genuinely recompile, and mutation-proved to reach struct fields and enum variants. | — |
| 2 | `core/wire.ts` + `core/ws-client.ts` | 157 |
| 3 | `core/modules.ts` + `core/merge.ts` + `core/contributions.ts` + `core/manifest.ts` | 127 |
| 4 | `core/actor.ts` + `core/hooks.ts` + `core/user-rest.ts` + `core/scene-docs.ts` + `core/mock-server.ts` | 106 |
| 5 | remaining `client/core` (loader, sheets, optimistic, chat-docs, services, templates, store, middleware, e2e/server-process, tail) | ~102 |
| 6 | `render/engine.ts` + `render/types.ts` | 131 |
| 7 | remaining `client/render` (backend.mock, pixi-backend, token-animator, grid, lighting, ping-view, layers, token-view, tail) | ~154 |
| 8 | `modules/panels` | 162 |
| 9 | `client/ui-kit` | 89 |
| 10 | `client/shell` | 85 |
| 11 | `scene-tools` 49 + `actors` 43 | 92 |
| 12 | `formula` 38 + `chat` 29 | 67 |
| 13 | tail: `chat-card`, `entry`, `sheet-actor`, `stage`, `sheet-item`, `chat-composer`, `assets`, `sheet-fallback`, `settings`, both `examples/` | 53 |
| 14 | the 6 named arrow/fn-expression sites | 6 |
| 14b | whatever Task 1's Rust proof surfaces (0 if the 0 holds; otherwise scoped then) | ? |
| 15 | **Ship task.** Ratchet `eslint.props.config.js` to `error` for every package in both its blocks; consolidate `eslint.docs.config.js` AND `eslint.props.config.js` into `eslint.config.js`; mutation-prove all tiers; full gate matrix; docs sync; reviewed skill-update gate; skills documentation-reference pass. **Also close the `ClassDeclaration` content gap** — see below. | — |
| 16 | **Core-skill commenting-lessons task** (user directive 2026-08-05). Fold the campaign's accumulated commenting rules and lessons into `shadowcat-codebase-core`. Runs AFTER Task 15 so it captures the finished rule set. Reviewed by `shadowcat-spec-reviewer` per the skill-update gate. Detailed below. | — |
| R15 | **Rule 15 conversion pass** (user directive 2026-08-05), executed out-of-band during Task 5's review window. Convert every committed `file:line` and bare-filename citation to a symbol citation across `src/**` doc comments and the live tracking docs. Measured: 151 `file:line` sites + ~170 bare-filename sites. Five file-disjoint agents. Excluded `src/client/core` (Task 5's re-reviews were pinned to it) and dated records under `docs/superpowers/`. | ~320 |

**`ClassDeclaration` content is unchecked by the shipped function gate, at zero current cost.** The
same `contextDefaults` mechanism described above means `eslint.docs.config.js` never content-checks a
`ClassDeclaration`: a class carrying an empty `/** */` passes `lint:docs`. Measured repo-wide during
Task 1: **0 live instances** — every documented class already carries a real description, so sweep
12's 154 → 0 stands and this is not a defect in shipped work. It is a hole in the instrument, not in
the docs. Task 15 closes it when it merges the configs, by giving `require-description` an explicit
context list that includes `ClassDeclaration`. Do NOT "fix" `MethodDefinition` alongside it —
methods are already fully content-checked, because a method's value node is a `FunctionExpression`,
which IS in the plugin's default list. That was verified by probe after being reported as a defect.

## Task 16 — fold the campaign's commenting lessons into `shadowcat-codebase-core`

**Why it is its own task.** Thirteen sweeps have produced a rule set that exists only in
`docs/design/doc-sweep-truthfulness-rules.md`, which is handed to doc-sweep implementers by path and
read by nobody else. Every ordinary coding task in this repo writes comments, and none of them load
that file. `shadowcat-codebase-core` is the always-invoked base skill — it is the only artifact that
reaches every agent, so it is where the durable commenting rules have to live.

**Ordering.** Runs AFTER Task 15, so it captures the finished rule set rather than a snapshot that
Task 15 then invalidates. Task 15's "skills documentation-reference pass" is a *different* job —
that one adds api/rust + api/ts + guide/protocol links to every skill's Pointers. This one adds
rules. Do not merge them.

**Scope — a compact rules block in the core skill's Gotchas, one line per rule, pointing INTO the
full document rather than restating it** (the skill family's own authoring rule: orientation+index,
never duplicate). The load-bearing set, by measured yield:

- **NEVER work around a rule — follow its INTENT; if unsure, ASK** (user directive 2026-08-05,
  iron-clad). Added inline ahead of this task. It outranks every rule below, because those rules are
  what get worked around. Reworking text until a rule stops applying is never acceptable, however
  true the result and however green the gate.
- **RULE 15 — cite symbols, never file names or line numbers.** Already added inline ahead of this
  task, since the R15 conversion pass needed it live. Task 16 keeps it and adds the rest.
- **RULE 1 — a citation must support the claim AS WORDED**, not an adjacent fact. The campaign's
  single highest-yield rule.
- **RULE 3 — no third copy.** A property whose semantics its enclosing type already states points at
  it; it does not restate it. Drift between copies is undetectable by any gate.
- **RULE 4 — prefer deletion over recomposition**, and delete the whole overclaim. Each rewrite is a
  fresh chance to assert something unverified.
- **RULE 5 — absolutes concentrate the errors** (never/always/only/sole/all). Enumerate before
  writing one.
- **RULE 9 — never append a second JSDoc block.** Tooling resolves to the nearest preceding block, so
  an appended one silently orphans the richer original while the linter reports green.
- **RULE 14 — a green `lint:docs` means tags are PRESENT, not that comments are TRUE**, and it counts
  only `/** */`. Standalone `//` and `const` comments are ungated and are where several of the
  campaign's best findings lived.

**Also mirror the durable subset into `docs/design/ARCHITECTURE.md`** — `CLAUDE.md` is git-ignored, so
a rule that lives only there does not survive a clone. ARCHITECTURE.md is the tracked source of truth.

**Gate.** Reviewed skill-update gate applies: dispatch `shadowcat-spec-reviewer` on the skill diff to
confirm it captures the rules without drift or broken pointers. One clean pass is not sufficient
evidence on its own — that gate has previously shipped two real errors after passing.

**The per-task site counts above are the PROPERTY gap only.** The 102 type-declaration sites live in
the same files and are absorbed by whichever task owns each file — `wire.ts` (Task 2) carries the
heaviest concentration, since the `z.infer` aliases are exactly that construct. Every brief
re-measures its own scope under the final config before dispatch and states the live number; **no
brief inherits a count from this table.** That rule has caught a defect in every sweep it ran in.

Tasks 2–14 are file-disjoint from each other, but **only one implementer runs at a time** — the
sweep-12 rule stands: never commit while a review is outstanding, because a verdict is pinned to the
commit the reviewer read.

## ⚠ This sweep ROTS doc→code citations. Every task must repair its own.

Adding doc comments shifts the code beneath them. Every citation pointing **from** a Markdown doc
**into** a file this sweep touches silently goes stale, and **no gate catches it** — neither
`lint:docs` nor `lint:props` parses Markdown, so a rotted pointer survives a fully green run.

Discovered at Task 5, by which point **eight** citations in `docs/OPEN_BUGS.md` had already rotted:
four from the sweep itself (`wire.ts` 192→234 and 359→424 in Task 2; `assets.ts` 59-68→76-90 and
41-45→58 in Task 5) and four from a single unrelated +5-line comment fix in `sqlite.rs`. Repaired in
`8fb44f2`.

**RULE 15 retires this problem rather than managing it.** The per-task repair check above was the
containment strategy; converting every live citation to a symbol is the fix. Once the live tracking
docs cite `AssetResolver.url` instead of `assets.ts:41-45`, an inserted comment block cannot
invalidate them — there is no coordinate left to shift.

**So the standing per-task instruction is now:** if a task touches a file that any live doc still
cites by `file:line`, convert that citation to a symbol rather than re-aiming it. Report the count
converted. Re-aiming a line number is repairing a defect by reproducing it.

The two traps below applied to the re-aiming strategy and are recorded because they explain why
re-aiming was never sustainable:
- **A citation that still RESOLVES may no longer SUPPORT.** After a shift, `wire.ts:192` landed on a
  JSDoc line — a real line, entirely the wrong one. A checker cannot classify this; only the prose's
  own wording decides which construct it meant.
- **An insertion cannot move a line ABOVE it.** Citations above the shift point were correct and had
  to be left alone — so the repair pass itself had to reason about direction, and "correcting" an
  already-correct citation was a live way to introduce the defect.

## Global constraints (bind every task)

1. **Comment-only.** No runtime change. Report a real defect with reachability bounded rather than
   fixing it; log it to `docs/OPEN_BUGS.md`. Correcting a **stale** comment is not a runtime change
   and IS in scope (Rule 7).
2. `docs/design/doc-sweep-truthfulness-rules.md` — all 15 rules, required reading per task.
3. **RULE 15 — cite SYMBOLS, never file names or line numbers.** Shipped prose names the type and
   member (`AssetResolver.url`, `Conn::handle_scene_subscribe`), qualified by its **owner** rather
   than its location. A `file:line` in a committed comment is now a defect in its own right,
   independent of whether it currently resolves. This supersedes the former path-qualification
   constraint, and it retires the line-delta problem at the root rather than policing it per task.
4. **A citation that RESOLVES is not one that SUPPORTS the claim** — the campaign's dominant defect.
5. **Scope every absolute.** "never/always/only/sole" must be enumerable from code.
6. **Rule 3 — no third copy.** A property whose semantics are stated by its enclosing type's doc
   points at it; it does not restate it. This matters far more here than in sweep 12: 1222 adjacent
   sites is the highest-pressure environment for copy-drift the campaign has faced.
7. **Rule 14 re-scan** — inventory every comment, including `//` and `const`, and report the count
   as a number.
8. `docs:check-examples` must stay at its entry value unless a task's brief names a different exit
   value and the one fence causing the delta. `.svelte` is structurally unscannable
   (`scripts/extract-ts-examples.mjs:99`), so no `.svelte` edit can move it.

## The property-doc quality bar

A property doc that says `/** The user id. */` above `user_id: string` is worthless and 1222 of them
would be actively harmful — noise that buries the real invariants. The bar per site:

- **State the invariant or the coupling, not the name.** `/** Server-assigned; stable across a
  replace, which swaps bytes behind it. */` beats `/** The asset uuid. */`.
- **Where a field is nullable, say what null MEANS** — the sweep-12 `hyperlinks` vs `link_previews`
  case is the template: same type, different meaning for `null`.
- **Where a field mirrors a server type, cite it path-qualified** and make it a Rule 6
  condition-by-condition claim, not a vibe match.
- A field whose meaning is genuinely exhausted by its name and type gets one short clause, not a
  manufactured paragraph. Padding is a defect.

## Exit criteria

- `pnpm lint:docs` **0** repo-wide (already true entering this sweep) and `pnpm lint:props` **0**
  repo-wide with every package ratcheted to `error` in all of `eslint.props.config.js`'s blocks,
  then both configs merged into `eslint.config.js` at Task 15.
- Both gates mutation-proven: an undocumented function (`eslint.docs.config.js`) AND an undocumented
  property (`eslint.props.config.js`) each report.
- `docs:check-examples` green at its final value.
- Full local gate matrix: `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`, `cargo fmt --check`,
  `cargo clippy --all-targets`, `cargo test`, Playwright e2e.
- Per-crate buddy-check convergence: fix + re-check each crate until clean.
- Reviewed skill-update gate: `shadowcat-codebase-*` skills updated and spec-reviewed.
- Merge to `main` `--ff-only`, push, watch CI.
