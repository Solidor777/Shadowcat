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
| Type declarations | `TSInterfaceDeclaration`, `TSTypeAliasDeclaration`, `TSEnumDeclaration` | **101** | `eslint.probe.config.js` (1324 combined) |
| **TS gate total** | | **1329** | |

Each probe's raw output is one higher than the real site count: both report the same "unused
eslint-disable directive" at `core/src/hooks.ts:25`, which is a `linterOptions` artifact of the
probe configs (they omit the shipped config's `reportUnusedDisableDirectives: false` block for that
one file, `eslint.docs.config.js:91-94`) and not a missing doc. The real config keeps that block, so
the artifact disappears once the contexts land in the new property-gate config (see Task 1 below).

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

| # | Scope | Sites |
|---|---|---|
| 1 | **Config + measurement task.** Create `eslint.props.config.js` (see above) with a `rulesAt`-shaped severity function gating the 4 property contexts, the 3 type-declaration contexts, and the 4 arrow contexts, at `warn` repo-wide to start. Add a `lint:props` script. Mutation-prove each of the 11 fires at `warn`. Then prove the Rust side: confirm `clippy::missing_docs_in_private_items` genuinely runs (not cached) and genuinely fires (mutation), and report the true Rust number. No ratchet yet, no doc writing. | — |
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
| 15 | **Ship task.** Ratchet `eslint.props.config.js` to `error` for every package in both its blocks; consolidate `eslint.docs.config.js` AND `eslint.props.config.js` into `eslint.config.js`; mutation-prove all tiers; full gate matrix; docs sync; reviewed skill-update gate; skills documentation-reference pass. | — |

**The per-task site counts above are the PROPERTY gap only.** The 101 type-declaration sites live in
the same files and are absorbed by whichever task owns each file — `wire.ts` (Task 2) carries the
heaviest concentration, since the `z.infer` aliases are exactly that construct. Every brief
re-measures its own scope under the final config before dispatch and states the live number; **no
brief inherits a count from this table.** That rule has caught a defect in every sweep it ran in.

Tasks 2–14 are file-disjoint from each other, but **only one implementer runs at a time** — the
sweep-12 rule stands: never commit while a review is outstanding, because a verdict is pinned to the
commit the reviewer read.

## Global constraints (bind every task)

1. **Comment-only.** No runtime change. Report a real defect with reachability bounded rather than
   fixing it; log it to `docs/OPEN_BUGS.md`. Correcting a **stale** comment is not a runtime change
   and IS in scope (Rule 7).
2. `docs/design/doc-sweep-truthfulness-rules.md` — all 14 rules, required reading per task.
3. **Path-qualify every citation. Re-measure by grep AFTER the last edit.** Never compute a line
   delta. A property doc added mid-file shifts every citation below it, including cross-file ones.
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
