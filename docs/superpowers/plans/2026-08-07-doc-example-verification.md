# Doc-example verification: close the untagged-fence blind spot

## Context

`pnpm docs:check-examples` extracts every ` ```ts ` fence inside a `@example` block,
writes each to a standalone module under `.docs-tmp/examples/`, and typechecks the
directory. It reports `333 TS doc examples typecheck OK`.

333 is the whole verified surface. A further **442 `@example` blocks are never
compiled**:

| Source | Tagged ` ```ts ` (checked) | Untagged (unchecked) |
|---|---|---|
| `.ts` under `src/`, `examples/` | 333 | 328 |
| `.svelte` under `src/` | 0 | 114 |

The extractor's header calls untagged fences "ignored by design". That is accurate as
a description of the code and false as a description of intent: nothing enforces that
an untagged fence is untagged *for a reason*, so the tag correlates with nothing.

### Why they are untagged

The standalone-module extraction model cannot compile an example that calls a
non-exported symbol. The dominant untagged form is exactly that:

```
// internal helper; not part of the public API (see resolveTokenActor for the public entry point)
project(actorDoc, actorDoc.engine as ActorEngine, token.engine?.overrides);
```

A probe run — 48 `@shadowcat/core` examples, each appended to a copy of its own module
and compiled against the package's real tsconfig — produced **zero** unresolved-symbol
errors for the documented helpers. `project`, `bandsTree`, `revertChild` and their
peers all resolve when compiled in-module.

What the probe surfaced instead is the real defect. About half the probes failed on
undeclared setup identifiers:

```
error TS2304: Cannot find name 'actorDoc'.
error TS2304: Cannot find name 'someWireDocument'.
error TS2304: Cannot find name 'mergedBands'.
```

These examples name inputs that do not exist anywhere. They are fragments. The
untagged fence is what has kept that invisible.

### Approach

Compile each example **in its own module's lexical context** instead of as standalone
consumer code, and stop treating fence tagging as an opt-in. This matches the stance
the project already takes on the Rust side, where `docs:api:rust` passes
`--document-private-items`: documenting internals is established policy, so the
instrument verifies internals rather than refusing them.

Rejected: requiring every example to compile as an outside consumer. It would force
deleting or relocating every private-helper example and contradicts that stance.

## Global Constraints

1. **No suppressions.** `@ts-ignore`, `@ts-nocheck`, `eslint-disable` of
   `no-unused-vars`, and Rust `allow`/`expect` are forbidden without the user's
   explicit per-instance approval, enforced by `pnpm lint:allowances`. An example
   that will not compile is a fix or an escalation, never a suppression.
2. **Rule 15 — cite symbols, not locations.** Comments and committed docs cite type
   names and members. Never file paths or line numbers. Dated files under
   `docs/superpowers/` are exempt.
3. **Rule 16 — code carries no ephemera.** No dates, task IDs, plan references, sprint
   names, or change history in code or comments, including this plan's name. Present
   tense, current state only.
4. **The tree must be left clean.** No generated probe or scratch file may survive a
   run, including a failed or interrupted one. `git status --short` is empty
   afterwards.
5. **Never permanently delete.** Use `trash` (relative paths — trash-cli silently
   no-ops on absolute Windows paths) or `send2trash`. `rm`, `Remove-Item`, `del` and
   kin are banned outside committed CI-only scripts.
6. **The instrument states its own coverage.** Any category the gate cannot check must
   be reported by the gate as an explicit count, never passed over in silence. A
   scope that matches zero files exits non-zero.
7. **Positive control required.** Every extraction predicate ships with a specimen that
   proves it detects what it claims, and the control is falsified (deliberately broken
   once) to prove it can fail. An unfalsified control is decoration.

## Task 1 — In-module example compiler

Rewrite `scripts/extract-ts-examples.mjs` so examples compile inside their own module.

**Extraction.** Collect every fence inside a `@example` block from `.ts` sources under
`src/types`, `src/client`, `src/modules`, `examples` (excluding `*.test.ts`,
`node_modules`, `dist`, `generated`) — both ` ```ts ` and untagged. Strip the leading
`* ` continuation from each line. Preserve the existing `\r?\n` handling: a CRLF
working copy must not silently match nothing.

**Compilation.** For each example, build a virtual source file whose text is the full
text of its host module followed by the example body wrapped in an `async` function
that is referenced once so `noUnusedLocals` stays satisfied. Place the virtual file at
a path in the host module's own directory so its relative imports resolve identically.

Use the TypeScript compiler API with a `CompilerHost` that overlays these virtual files
in memory. **Do not write probe files into the source tree** — an interrupted run would
leave them behind, violating constraint 4. Resolve each package's real options through
`ts.parseJsonConfigFileContent` on its own `tsconfig.json` so examples are checked under
the same `strict`, `noUnusedLocals` and `verbatimModuleSyntax` settings as the code they
document.

**Reporting.** On failure, map every diagnostic back to the host module symbol and the
example's ordinal within it, and print that mapping — a diagnostic pointing only at a
virtual path is unactionable. On success print the count of examples compiled. Report
the `.svelte` example count as an explicitly unchecked category (Task 3 covers it).

**Controls.** Per constraint 7, ship specimens covering: a tagged fence, an untagged
fence, a fence whose body calls a non-exported symbol, a CRLF-delimited fence, and a
fence inside a block comment that is not a doc comment (must not be collected). Run
them on import and exit non-zero on mismatch. Falsify each once during development and
confirm it fails.

Expect this task to turn the gate red. That is the point; do not fix examples here.

**Verify:** `node scripts/extract-ts-examples.mjs` runs to completion and reports a
failure count. `git status --short` is empty. Existing `scripts/` unit tests pass via
`pnpm test:scripts`.

## Task 2 — Failure census

Add a `--json` mode emitting one record per failing example: host module, host symbol,
ordinal, and the diagnostics. Group and report by package.

This is the input to the repair tasks, which are enumerated once the true per-package
counts are known. Do not repair anything in this task.

**Verify:** `node scripts/extract-ts-examples.mjs --json` emits valid JSON; the record
count equals the failure count from Task 1.
