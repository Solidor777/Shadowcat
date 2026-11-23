# Shadowcat Virtual Tabletop — Agent Instructions

## Project
- Open source virtual tabletop. Hostable locally as a single executable. Supports custom modules, systems, UI rewrites, and mechanics.
- Fully modular: the platform is built to be modded. Support modding as first class.
- Stack: Rust, Svelte 5 (Runes), SCSS.
- All source resides in `src/`. Build output goes to `dist/`.

## Reference Docs
| Doc | Role |
|---|---|
| `docs/design/ARCHITECTURE.md` | Source of truth: engine invariants, technology choices, code style, testing rules |
| `docs/PLAN.md` | Forward roadmap: only unbuilt work |
| `docs/HISTORY.md` | Completed milestones + work log (historical artifact; append-on-completion only) |
| `docs/OPEN_BUGS.md` | Currently open bugs |
| `docs/TODO.md` | Deferred-work backlog |
| `docs/POST_WORK_FINDINGS.md` | Living record of post-work review issues. NOT a to-do list |
| `docs/design/` | Per-system design documents |

## Validation
- Work is done only when both tiers pass:

```bash
pnpm gate:commit
pnpm gate:push
```

- `scripts/gates.toml` lists what each tier runs. `pnpm lint:gate-manifest` fails when it and `ci.yml` disagree.
- Every gate is retroactive. A violation already in the tree is a defect to fix, not a precedent.

| Gate | Fails when |
|---|---|
| `cargo fmt --all -- --check` | Rust code is not formatted |
| `cargo clippy --all-targets -- -D warnings` | any clippy warning |
| `cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items` | a Rust item, public or private, has no doc |
| `cargo test --all`, `pnpm -r test`, `pnpm run test:scripts` | a test fails |
| `pnpm -r typecheck` | a TypeScript type error |
| `pnpm lint` | an ESLint error |
| `pnpm lint:docs` | a function, method or class lacks a doc comment with description, params and an `@example` |
| `pnpm lint:props` | a property, type, enum member or named arrow lacks a doc comment |
| `pnpm lint:comments` | a code comment or codebase-skill brief names a milestone, task, date, plan or other process-assigned thing |
| `pnpm lint:allowances` | a lint suppression is not approved (see Lint Suppression) |
| `pnpm lint:file-size` | a file passes the size limits (see File Size) |
| `pnpm lint:inline-tests` | a Rust file holds an inline test module body |
| `pnpm lint:module-docs` | a Rust `mod` item carries an outer doc comment |
| `pnpm lint:secure-context` | client code names a browser API that plain-HTTP LAN play does not expose |
| `pnpm lint:route-prefix` | client code writes a same-origin `/api/`, `/modules/` or `/ws` path as a bare string instead of through `apiPath` or `wsUrl` |
| `pnpm lint:walk-sites` | a directory-walking call exists that `scripts/walk-sites.json` does not classify, or the inventory names a site that no longer exists |
| `pnpm run check:svelte-runtime` | a file imports a `svelte` entry point the runtime does not enumerate |
| `pnpm lint:crypto-backend` | a second TLS crypto backend enters `Cargo.lock` |
| `pnpm lint:vendor` | project code hides under `vendor/`, or a vendored crate drifts from its recorded upstream plus patch |
| `pnpm lint:aria-labels` | a human-facing attribute carries an i18n key instead of translated text |
| `pnpm lint:gate-manifest` | `scripts/gates.toml` and the workflow steps disagree |
| `pnpm lint:settings-privacy` | a tracked file under `.claude/` carries a user path, an address or a credential |
| `pnpm lint:binary-size` | the release binary passes 60 MiB |
| `pnpm lint:audio-fixtures` | the committed long-audio fixtures differ from what the generator produces |
| `pnpm docs:check-examples`, `pnpm docs:check-rust-examples` | a doc example does not compile, or a Rust function has none |
| `pnpm build:all` | the build or the strict doc build fails |
| `git diff --exit-code src/types/generated` | the generated types are stale |

## Cross-Platform
- Every artifact runs on macOS, Linux, and Windows (the server binary) and renders correctly in desktop and mobile browsers (Android, iOS) from the first commit. Cross-platform is a build-time invariant verified in CI, not a later port.
- Code that compiles only on the author's OS, or a UI that assumes a mouse and a wide viewport, is a defect.

### CI Matrix
- Build and test the server on macOS, Linux, and Windows via a CI matrix. A green pipeline on one OS is no evidence for the others.
- **Good:** `matrix: os: [ubuntu-latest, macos-latest, windows-latest]` with `runs-on: ${{ matrix.os }}`
- **Bad:** `runs-on: ubuntu-latest` alone

### Portable Paths
- No hardcoded path separators, drive letters, or OS-specific directories. Build paths with `std::path` (`Path::join`, `PathBuf`) and resolve user, config, and data locations through a portable abstraction.
- Gate genuinely OS-specific code behind `#[cfg(...)]` with an implementation for every target.
- **Good:** `dir.join("shadowcat.db")`, `config_dir().join("shadowcat").join("cfg.toml")`
- **Bad:**
  ```
  format!("{dir}\\shadowcat.db")
  "/home/user/.config/shadowcat/cfg.toml"
  ```

### Portable Shell
- CI and build scripts must not depend on one OS's shell builtins or binary naming: no GNU-only flags (`stat -c`), assumed binary suffixes, or bash-only syntax.
- **Good:** `bin="target/release/shadowcat${{ runner.os == 'Windows' && '.exe' || '' }}"; size=$(wc -c < "$bin")`
- **Bad:** `size=$(stat -c%s target/release/shadowcat)`

### Mobile & Touch Client
- Every served HTML page declares a responsive viewport, layouts reflow to a phone screen, and interactive targets are touch-sized. No hover-only interaction.
- **Good:** `<meta name="viewport" content="width=device-width, initial-scale=1.0" />`
- **Bad:** a `<head>` with no viewport meta

## Collaboration

### Immutable History
- Never destroy, rewrite, or drop git commits. No `push --force`, `reset --hard`, or history-dropping rebases. Prefer `revert` or new commits.
- **Good:** `git revert a1b2c3d`
- **Bad:** `git reset --hard HEAD~3` then `git push --force origin main`

### Architectural Consent
- Before non-trivial work, summarize the intended change and verify it against `ARCHITECTURE.md`, `PLAN.md`, and the design docs.
- Dependencies, file layouts, and public APIs are architectural choices requiring consent.
- Never deviate from an agreed spec unilaterally.
- Treat a null result (blank canvas, empty output) as an implementation failure until the code shows otherwise. Assume a flawed implementation before a flawed spec.
- **Good:** "Proposed: migrate input parsing to async. Aligns with the non-blocking I/O rule in `ARCHITECTURE.md`. Consent required: add `tokio` and place the module in `src/net/`?"
- **Bad:** "I added `tokio` and split the network logic into three new crates."

### Complications
- Fix small, clear follow-ups inline, with unit tests.
- Defer only work that actively conflicts with the current goal or context limits, and log every deferral to `TODO.md`.
- Surface complications the moment you find them, not at the end of a run.
- **Good:** Mid-run: "Halting. Version conflict with `serde` during setup; resolving before implementation."
- **Bad:** End of run: "By the way, dependency resolution failed earlier, so half the tests are broken."

### Warnings & Precedence
- Warn explicitly when an instruction conflicts with the design or is flawed, and get confirmation before proceeding.
- The project `CLAUDE.md` supersedes any other instruction.
- **Good:** "Warning: the requested global singleton conflicts with the lock-free threading model in `CLAUDE.md` and will introduce race conditions. Confirm to proceed."
- **Bad:** Implementing the singleton immediately despite the conflict.

## Code Standards

### Lint Suppression
- Every suppression silences a diagnostic without fixing what it describes. No agent adds one on its own authority.
- `dead_code` and `unused*` suppressions are never approvable, as `allow`, as `expect`, or as `eslint-disable` of `no-unused-vars`. There should be no dead code, so there is no sign-off to ask for.
- Every other suppression needs the user's explicit, per-instance sign-off.
- Covered: `#[allow(dead_code)]`, `#[allow(unused*)]`, `#[allow(clippy::*)]`, `#[expect(...)]` of any of these, `eslint-disable` of `no-unused-vars`, `@ts-ignore`, `@ts-nocheck`.
- `pnpm lint:allowances` enforces this as a gate, not a ratchet: it is retroactive and nothing is grandfathered. A suppression already in the tree is a defect to fix, not a precedent.
- Fix the code, never move the annotation: make the item reachable, delete it, or scope it to the build that uses it (`#[cfg(test)]` for test-only code, which must never compile into the release binary). Refactor the signature rather than silencing `too_many_arguments`.
- If no fix exists, ask how to change the code. Never ask to register a `dead_code` or `unused*` suppression.
- **Good:** `pub truncated: bool,` threaded onto the wire frame so the lint no longer fires; `#[cfg(test)] pub probe_counter: u32,`; `fn execute(ctx: &MoveCtx, route: &Route) {}`
- **Bad:** `#[allow(dead_code)] pub truncated: bool,`; `#[allow(clippy::too_many_arguments)] fn execute(a: A, …, h: H) {}`

### File Size
- Over 5,000 lines is a defect; over 10,000 is a build failure. `pnpm lint:file-size` (`scripts/check-file-lines.mjs`) checks every tracked `rs ts js mjs svelte scss css` file under `src/`, `scripts/`, and `examples/` (generated types excluded). Test lines count. Same gate rules as Lint Suppression: retroactive, nothing grandfathered.
- Past 5,000 lines the gate fails unless `.claude/file-size-allowlist.toml` has an entry. Every entry needs the user's explicit, per-file authorization in the conversation. No agent adds, edits, or retains one on its own authority. A stale entry (file back under the limit) also fails the gate.
- Past 10,000 lines there is no override. Split the file.
- **Good:** Split the file.
- **Bad:** Adding `[[file]] path = "src/server/src/data/sqlite.rs"` "because the split is large."

### Rust Test Modules
- `pnpm lint:inline-tests` (`scripts/check-inline-tests.mjs`) fails on any inline `#[cfg(test)] mod x { … }` body in `src/**/*.rs`.
- Declare `#[cfg(test)] mod x;` and put the body in `<stem>/x.rs` (or `x.rs` beside a `mod.rs`). `use super::*` resolves to the same parent, so nothing widens.
- `#[cfg(test)]` on a non-module item (test-only helper, field, `impl`) stays in the production file.
- A test file that outgrows the limit splits by subject under `<stem>/tests/<subject>.rs`, with shared fixtures in `<stem>/tests/mod.rs`.
- **Good:** `#[cfg(test)] mod tests;` with the body in `data/sqlite/tests/mod.rs`
- **Bad:** `#[cfg(test)] mod tests { /* 7,000 lines */ }`

### Comments
- Optimize for machine context and exact state. Lead with load-bearing facts: invariants, constraints, hidden coupling. No narrative scaffolding or chatter.
- State what the code does or why it exists, in the present tense. Express historical reasons as present constraints.
- Banned: project history, bug-fix narration, task IDs, sprint references, spec documents, and narration addressed to reviewers. Agents cannot resolve external tracking systems.
- Cite the source and justify the choice for every algorithmic, rendering, or data-flow decision.
- Define technical terms inline. Document hidden coupling (what else breaks if this changes) and mathematical invariants.
- Use a `TODO:` prefix describing the technical work. No assignee, date, or ticket ID. Log every deferred item in `TODO.md`.
- Treat existing comments as stale until verified against the executing code. Update or delete any comment that contradicts changed logic.
- **Good:** `// Uses a Set to enforce O(1) lookups; duplicate entries degrade rendering performance.`
- **Good:** `// Algorithm: GGX microfacet BRDF. Source: [Walter et al. 2007]. Chosen over Beckmann for longer specular tails.`
- **Good:** `// Implicit coupling: depends on RenderPass::Begin clearing the output buffer to 0x00.`
- **Bad:** `// Previously we used an Array here, but it caused O(n) lookups in v2.1.`
- **Bad:** `// Per TICKET-842: add padding to the struct.`
- **Bad:** `// TODO(@dave): Refactor this phase later (Epic 5).`

### Comment Citations
- Cite type names and members, never source line numbers or source file names. A position goes stale without any error; a symbol name travels with the code.
- A file with no symbols (a manifest, a workflow) is named by path.
- **Good:** `// Mirrors resolve_token_footprint.`
- **Bad:** `// Mirrors footprint.rs line 412.`

## Security & IP
- Zero tolerance for intellectual property infringement and data leaks. System state, test fixtures, and debug dumps stay free of proprietary code, credentials, and PII.

### Clean-Room Implementations
- Never ingest, reproduce, or reference proprietary, copyrighted, or leaked code. Never mention proprietary software in source files.
- When an algorithm is required, implement it clean-room from academic papers, standard formulas, or public API documentation, and cite the source.
- **Good:** `// Algorithm: Ray-AABB intersection. Source: [Majercik et al. 2018].`
- **Bad:** `// Implementing the physics identical to Havok engine.`

### Secrets & PII
- Code, config, tests, and debug dumps contain no PII, real user data, internal corporate emails, or hardcoded secrets. Inject secrets via environment variables or a credential manager.
- Test fixtures and seed data are deterministic, structurally valid, and obviously synthetic. Never use subsets of production data. Use reserved test values that cannot reach real systems (`example.com` and `.example` domains per RFC 2606, `555` numbers).
- **Good:**
  ```
  process.env.TEST_API_KEY
  "testuser-01@example.com"
  "name": "MOCK_USER_A", "ssn": "000-00-0000"
  ```
- **Bad:**
  ```
  "sk-proj-9876543210abcdef"
  "sarah.jenkins@acmecorp.com"
  "name": "David Smith"
  ```

### Sanitization on Contact
- If workspace context, logs, or instructions contain PII or secrets, scrub them immediately. Never echo them into tracking files (`POST_WORK_FINDINGS.md`, `OPEN_BUGS.md`), `debug/dumps/`, or commit messages.
- **Good:** "API failed on mock customer record due to malformed date parser. [PII redacted from original log]."
- **Bad:** "API failed when querying customer Jane Doe (DOB: 05/12/1984) with credit card ending in 4111."

## Workflow

### Tracking Files
- The last step before clearing or compacting context is updating every tracking file. The plan is incomplete until they are current.
- `PLAN.md`: move completed items to Completed, pointing at the memory file.
- `TODO.md`: log deferrals, trimmed and optimized. Bugs never go here.
- `OPEN_BUGS.md`: add new bugs and remove verified-fixed ones. `CLOSED_BUGS.md`: log each resolution.
- `POST_WORK_FINDINGS.md`: log mid-run anomalies.
- Clear stale build artifacts from `target/`.
- **Bad:** `TODO.md` containing "BUG: Game crashes when pressing Esc."
- **Good:** `OPEN_BUGS.md`: "[Crash] Unhandled panic on Esc keydown during main menu transition."

### Debug Dumps
- Write all debug artifacts to `debug/dumps/` at the workspace root, never to the repo root or `target/`.
- Trash dumps (`trash <path>`) once the bug family is resolved. Never commit them.
- **Good:** `fs.writeFileSync("./debug/dumps/trace_output.json", dump)`
- **Bad:** `fs.writeFileSync("./trace_output_final.json", dump)`; `git add debug/dumps/memory_leak_trace.ppm`

### Release Builds
- Debug code never reaches release builds. Before committing, strip temporary instrumentation: `dbg!`, debug `println!`/`eprintln!`, `console.log`, `debugger;`, commented-out scaffolding.
- Persistent diagnostics go through a leveled facility that is silenced or compiled out in release: Rust `tracing` levels (statically removable via `release_max_level_*`), `debug_assert!`, `#[cfg(debug_assertions)]`. Client logging uses the project logger, never raw `console.log`.
- **Good:** `tracing::debug!(?user, "resolving access")`, `debug_assert!(doc.is_valid(), "INVARIANT: …")`, `logger.debug("applying optimistic op", op)`
- **Bad:** `println!("DEBUG resolve_access user={user:?}")`, `dbg!(&doc.permissions)`, `console.log("op", op); debugger;`

### Commits & Pushes
- Commit each logical work-unit as soon as local CI passes, without asking. Don't batch unrelated concerns into one commit.
- Push only when a full milestone completes, then watch CI (`gh run watch`).
- Enforcement: tracked git hooks (`core.hooksPath`) run `pnpm gate:commit` on every commit and verify a tree-keyed `pnpm gate:push` receipt on every push. There is no `--no-verify` bypass.
- The commit hook skips only while a rebase, cherry-pick, revert, or merge is genuinely mid-operation (a queued step in its todo file, or an unresolved conflict in the index). A stale or abandoned operation still prints a message naming the command to conclude it.
- Run `pnpm gate:push` before pushing. A commit made after the last successful run invalidates the receipt.
- **Bad:** `git push origin main` after a sub-task rather than a full milestone.

### Plan Execution
- Once a multi-phase plan is approved, drive it task to task without asking permission to continue.
- Task size, session length, and context usage are never reasons to stop.

### Crashes & CI
- Work is never complete with a crash or a red pipeline.
- On a crash, read the latest panic trace yourself, fix the root cause, and re-run.
- If CI goes red after a push, fix forward layer by layer, starting from the topmost error. Pause only if genuinely stuck.
- **Good:** "Build panicked. Read `debug/dumps/panic.log`: null pointer dereference in `RenderPass`. Fixing the initialization sequence and re-running tests."
- **Bad:** "It panicked. Let me know how to proceed."

## Graphify
- The knowledge graph at `graphify-out/` holds god nodes, community structure, and cross-file relationships.
- For questions touching multiple files or crates, when `graphify-out/graph.json` exists, first run `graphify query "<question>"`. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. They return a scoped subgraph far smaller than `GRAPH_REPORT.md` or raw grep output.
- If `graphify-out/wiki/index.md` exists, use it for broad navigation instead of raw source browsing.
- Read `GRAPH_REPORT.md` only for broad architecture review or when query/path/explain surface too little.
- After modifying code, run `graphify update .` (AST-only, no API cost).

## Codebase Skills & Agents

### Plugin
- Codebase knowledge lives in the standalone `shadowcat-codebase` plugin (`github.com/Solidor777/shadowcat-codebase`), checked out at `~/.claude/skills/shadowcat-codebase/` (the `skills-dir` convention: auto-loaded live every session, edits visible next session, no version bump).
- It is installed at user scope and disabled by default everywhere. Each repo opts in with `claude plugin enable shadowcat-codebase@skills-dir --scope local` (gitignored, per-machine).
- Address every skill and agent with the `shadowcat-codebase:` prefix (`shadowcat-codebase:shadowcat-codebase-core`, `shadowcat-codebase:shadowcat-coder`), never the bare name.
- `shadowcat-codebase-core` is the always-relevant base skill, with one `shadowcat-codebase-<subsystem>` skill per subsystem. Read the current set from core's **Subsystem skills** list, never from an enumeration elsewhere.
- Each skill is an orientation and index brief (Purpose / Key files / Hard invariants / Gotchas / Pointers) that routes into graphify, `docs/design/`, and memory rather than duplicating them.
- The plugin's `hooks/hooks.json` auto-wires the `Edit|Write|MultiEdit` activation hook when enabled. Subagents don't get it and must invoke skills explicitly.
- Two local-only scripts verify the skill corpus: `scripts/check-skill-symbol-refs-cli.mjs` (code-symbol citations) and `scripts/check-skill-api-refs-cli.mjs` (generated-doc pointers). Run them by hand. Skills root: `defaultSkillsRoot()` in `scripts/lib/gate-corpus.mjs`, overridable with `SHADOWCAT_CODEBASE_SKILLS_DIR`. CI has no access to `~/.claude/`, so they are not in `ci.yml`.
- `pnpm run test:scripts` is CI-enforced. Tests needing a real skills checkout skip when `defaultSkillsRoot()` doesn't exist.

### Skill Update Gate
- Whenever a plan finishes execution, or an inline change alters a subsystem's seam, invariant, or gotcha, update the affected skill(s) in the plugin checkout (`~/.claude/skills/shadowcat-codebase/skills/...`) before merge or clear. This gate blocks completion at the same tier as Tracking Files.
- If work opens a subsystem no skill covers, create a new `shadowcat-codebase-<subsystem>` skill there (fixed shape; add its globs to the `SUBSYSTEMS` map in `hooks/codebase-skill-reminder.py`).
- Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` to confirm each skill diff captures the change (no omission, drift, or broken pointer).
- Trivial changes touching no subsystem knowledge need no edit, but say so explicitly.
- Run `node scripts/check-skill-symbol-refs-cli.mjs` locally after any skill edit. It verifies every citation in the corpus, not just the skill touched.
- Commit and push skill edits from inside the checkout. It is a separate repository (`Solidor777/shadowcat-codebase`), not part of any commit here.
- A change to the body of `shadowcat-coder`, `shadowcat-code-reviewer`, or `shadowcat-spec-reviewer` must be mirrored to that agent's `-opus` and `-fable` twins in the plugin's `agents/` directory.
- **Good:** "Plan done. Updated `shadowcat-codebase-actors-tokens` (new faction-border seam + invariant). `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff: PASS. `check-skill-symbol-refs-cli.mjs`: 0 broken. Committed and pushed in the plugin repo. Merging here."
- **Bad:** "Plan done, merging." (factions added; skill never updated or reviewed)

### Agent Dispatch
- Subagents don't auto-activate skills. Use the plugin's agents, each of which invokes the relevant skill first.
- Delegating implementation: `shadowcat-codebase:shadowcat-coder`.
- Any review checkpoint (buddy-check, `requesting-code-review`, mainline-plan-execution final review): `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer` as a pair.
- Reviewers can write one review file and nothing else. Name the file in the dispatch when a durable copy is wanted; the final message is still the whole report.
- **Good:** `Task(shadowcat-codebase:shadowcat-coder, "implement the faction border")`
- **Bad:** `Task(general-purpose, "implement the faction border")`

### Effort & Escalation
| Agent | Effort | Escalation twin |
|---|---|---|
| `shadowcat-codebase:shadowcat-coder` | medium | `shadowcat-codebase:shadowcat-coder-opus` |
| `shadowcat-codebase:shadowcat-code-reviewer` | high | `shadowcat-codebase:shadowcat-code-reviewer-opus` |
| `shadowcat-codebase:shadowcat-spec-reviewer` | high | `shadowcat-codebase:shadowcat-spec-reviewer-opus` |

- Never pass `model` or `effort` on a dispatch to these agents: each agent's frontmatter fixes them (the table gives the effort each runs at).
- Each twin is `model: opus`, `effort: high`, with an identical body.
- On BLOCKED, or reviewer findings that read as shallow or uncertain, re-dispatch to the twin before involving the human.
- Work outside these three agents' scope uses the global `sdd-*` agents.
- **Good:** Coder dispatched with no model or effort passed (the agent file sets medium); on BLOCKED over an ambiguous ownership model, re-dispatched to `shadowcat-coder-opus` before asking the user.
- **Bad:** Passing `effort` on the dispatch, then escalating straight to the user on BLOCKED.