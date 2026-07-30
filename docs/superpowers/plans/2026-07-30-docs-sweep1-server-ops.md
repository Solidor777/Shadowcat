# Docs Sweep 1 — Server-Ops Cluster Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution` (per project CLAUDE.md this replaces
> subagent-driven-development / executing-plans on Fable). On any other model use
> superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** First doc-comment sweep of the campaign: fully document the server-ops
cluster (`config.rs`, `db.rs`, `backup.rs`, `modules.rs`, `lib.rs`, `main.rs`,
`bin/test_server.rs`) — every item described, every function carrying a compiled
example — then flip the cluster's lints to deny so coverage can never regress.

**Architecture:** Docs-only sweep + lint attributes; zero behavior change. This
sweep CALIBRATES the patterns every later sweep copies: the doctest policy
(runnable vs `no_run` vs bin-crate `text`), the per-item-class doc shapes, and
the per-file deny-flip mechanics. Measured backlog: 51 undocumented items
(config 18, modules 12, backup 11, lib 8, main 1, test_server 1, db 0) plus
`# Examples` blocks for every function in scope.

**Tech Stack:** rustdoc doctests, `#![deny(missing_docs)]` +
`#![deny(clippy::missing_docs_in_private_items)]` inner attributes, existing CI
gates (3-OS `clippy -D warnings`, `cargo test`).

**Spec:** `docs/superpowers/specs/2026-07-30-documentation-system-design.md`
(Phases 2–N section). Campaign state: `docs/PLAN.md` "Documentation campaign".

## Model/Effort directives

Authored mainline in the requesting session (Fable 5, effort high) under the
user's standing same-session directive ("you write the plan, no model switch").
Execution: `mainline-plan-execution` on this Fable session; ONE final
whole-branch review via the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`
pair.

## Buddy-check directives

No high-risk signals (docs + lint attributes only; no behavior change, no
security-sensitive paths). Standard final review only.

## Global Constraints

- **Docs-only sweep**: no behavior change anywhere. If writing docs surfaces a
  real bug, per the standing burndown directive fix it in its own separate
  commit (trivial) or log it to `docs/OPEN_BUGS.md` (non-trivial) — never fold
  a behavior change into a docs commit.
- Comment rules (project CLAUDE.md): present tense, invariants and hidden
  coupling first, cite algorithm/decision sources, no history narration, no
  process meta. Existing good docs in these files are the style baseline — match
  them, don't restyle them.
- **Doctest policy (the calibration pattern later sweeps copy):**
  - Public fn/method in the lib crate → `/// # Examples` with a fenced ` ```
    ` doctest. Runnable when pure (construct, call, assert); ` ```no_run ` when
    it touches DB/filesystem/network/tokio runtime. ` ```ignore ` is BANNED
    (spec §2).
  - Private fn/method → doctests compile as an external crate and cannot call
    private items: give the example THROUGH a public caller when one exists,
    else a ` ```text ` illustrative block. Never a fake doctest.
  - Bin crates (`main.rs`, `bin/test_server.rs`) → rustdoc runs no doctests for
    bin targets: crate-level `//!` docs carry ` ```text ` CLI invocation
    examples; fns get descriptions (+ ` ```text ` where an example clarifies).
  - Struct fields, enum variants, module decls → description only (the spec's
    example requirement targets functions: "every function, their parameters,
    and examples"). Parameters are described in the fn's doc prose per rustdoc
    convention.
- Every `@`/`#` count claim is re-verified at execution time with the scoped
  count command (below), never trusted from this plan.
- Scoped count command (used as each task's red/green gate), run from
  `src/server/` in a subshell:
  `cargo clippy --all-targets -- -W missing-docs -W clippy::missing-docs-in-private-items 2>&1 | grep -c "<file>.rs"`
  (target: 0 for the task's file(s); the `-W` run is warn-tier so it exits 0
  regardless — the COUNT is the gate).
- Each task ends green on: `cargo test` (doctests included), `cargo fmt --all
  --check`, `cargo clippy --all-targets -- -D warnings` — from `src/server/` in
  a subshell (cwd discipline). Commit per task.
- Work on a feature branch `docs-sweep1-server-ops` off main; push only at plan
  completion.

---

### Task 1: config.rs (18 items + examples on all methods)

**Files:**
- Modify: `src/server/src/config.rs`

**Interfaces:**
- Produces: the per-item-class doc patterns Tasks 2–4 copy.

- [ ] **Step 1: Enumerate the real backlog**

Run the scoped count command for `config.rs` — expect 18 (11 `Cli` fields at
lines 14–32, 5 `Config` fields at 51–57 (`bind`, `db`, `admin_user`,
`admin_password`, `session_key`), 2 `SetupTokenPolicy` variants at 118–119, and
`setup_token_policy` at 243). Re-derive the exact list from the clippy output,
not from these line numbers.

- [ ] **Step 2: Document the 18 items**

Class patterns (write real content per item; these are SHAPE examples):

```rust
pub struct Cli {
    /// Listen address override (`host:port`); wins over every other layer.
    #[arg(long)]
    pub bind: Option<String>,
```

```rust
/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
pub enum SetupTokenPolicy {
    /// `/api/setup` accepts the first admin without a token.
    Open,
    /// A token gates `/api/setup`; `None` = generate-and-log at boot.
    Required(Option<String>),
}
```

Every `Cli` field doc must state what the flag overrides; every `Config` field
doc must state its default and which env/TOML key reaches it (the figment
layering is already documented on the struct — don't repeat it per field, state
the field's own meaning/default).

- [ ] **Step 3: Add `# Examples` to every method**

`config.rs` methods: `Config::load`, `assets_path`, `modules_path`,
`backups_path`, `effective_max_bytes`, `effective_rate_per_min`,
`is_loopback_bind`, `setup_token_policy`. All are pure or fs-free → RUNNABLE
doctests. Pattern:

```rust
/// # Examples
///
/// ```
/// use shadowcat::config::{Config, SetupTokenPolicy};
///
/// let cfg = Config::default(); // loopback bind + setup_token "auto"
/// assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
/// ```
```

`Config::load` reads a TOML path + env → `no_run` (its doctest shows the call,
not its effects). Doctests must import via `shadowcat::...` (external-crate
view) and compile — `cargo test` runs them.

- [ ] **Step 4: Verify green**

From `src/server/` (subshell): scoped count for `config.rs` → 0; `cargo test`
(new doctests PASS, count them in the doctest summary line); `cargo fmt --all
--check`; `cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/config.rs
git commit -m "docs(server/config): document all items + doctests on every method"
```

---

### Task 2: backup.rs + db.rs (11 items + examples on all functions)

**Files:**
- Modify: `src/server/src/backup.rs`, `src/server/src/db.rs`

- [ ] **Step 1: Enumerate** — scoped counts: `backup.rs` expect 11 (6 manifest
  struct fields ~37–42, 5 error/enum variants ~22–30), `db.rs` expect 0
  (verify; it still needs Step 3's examples).

- [ ] **Step 2: Document the 11 items** — variant docs state the failure
  condition each variant represents; manifest field docs state what the value
  records and who reads it (restore-time validation, the printed summary line).

- [ ] **Step 3: Examples on every function** — `create_backup`,
  `restore_backup` (async + sqlite + fs) → `no_run` doctests showing a
  realistic call. `db.rs` has exactly one fn, `open_pool(url)` (already
  described): give it a RUNNABLE async doctest using `"sqlite::memory:"`
  (mirror its own unit test; `tokio_test`-free form: a `no_run` doctest is
  acceptable if the async runtime makes runnable awkward — doctests have no
  tokio main; use ` ```no_run ` with an async fn wrapper).
  `dir_is_empty_or_absent`-class pure helpers → runnable doctests via
  `std::env::temp_dir()` ONLY if deterministic; otherwise `no_run`. Private
  helpers → example through the public caller or ` ```text `.

- [ ] **Step 4: Verify green** — scoped counts 0 + the Task 1 Step 4 gate set.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/backup.rs src/server/src/db.rs
git commit -m "docs(server/backup,db): document all items + doctests"
```

---

### Task 3: modules.rs (12 items + examples on all functions)

**Files:**
- Modify: `src/server/src/modules.rs`

- [ ] **Step 1: Enumerate** — scoped count expect 12 (6 fields ~18–31, the
  struct at ~30, a function at ~34, 4 struct fields ~49–52).

- [ ] **Step 2: Document** — the discovery types' docs must carry the
  subsystem's load-bearing invariant where it lives: the install FOLDER name is
  the identity key (author-declared manifest ids are untrusted), and a missing
  modules dir scans as "no modules installed" (see the module-toolchain skill —
  the docs must agree with it, not paraphrase it into drift).

- [ ] **Step 3: Examples** — `scan_installed_modules(path)` returns empty for a
  missing dir → RUNNABLE doctest with a nonexistent temp path asserting
  `is_empty()`. Other fns per policy.

- [ ] **Step 4: Verify green** — scoped count 0 + full gate set.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/modules.rs
git commit -m "docs(server/modules): document discovery types + doctests"
```

---

### Task 4: lib.rs, main.rs, bin/test_server.rs (10 items)

**Files:**
- Modify: `src/server/src/lib.rs`, `src/server/src/main.rs`,
  `src/server/src/bin/test_server.rs`

- [ ] **Step 1: Enumerate** — lib.rs expect 8 (crate-level doc + 7 `pub mod`
  decls), main.rs expect 1 (crate doc), test_server.rs expect 1 (one field).

- [ ] **Step 2: lib.rs** — `//!` crate doc: what the `shadowcat` lib crate is
  (authoritative server: documents/permissions/realtime/scene under one
  embedded-client binary), one sentence per `pub mod` on its own `///`.

- [ ] **Step 3: main.rs + test_server.rs** — `//!` crate docs with ` ```text `
  CLI examples (serve mode, `--backup-to`, `--restore-from`; test_server's
  `--modules-dir` e2e harness role). Document the one bare field. Any fn in
  these bins still lacking a description gets one (bin doctests don't run —
  ` ```text ` only).

- [ ] **Step 4: Verify green** — scoped counts 0 for all three + full gate set.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/lib.rs src/server/src/main.rs src/server/src/bin/test_server.rs
git commit -m "docs(server): crate + module-decl docs for lib and bins"
```

---

### Task 5: Deny flip, verification, docs-sync, ship

**Files:**
- Modify: `src/server/src/{config,db,backup,modules,main}.rs`,
  `src/server/src/bin/test_server.rs` (inner deny attributes)
- Modify: `docs/PLAN.md` (campaign section: Sweep 1 complete)
- Modify: `.claude/skills/shadowcat-codebase-server-ops/SKILL.md` (ratchet note)

- [ ] **Step 1: Flip the cluster to deny**

Top of each of the six files (after the `//!` docs, before items):

```rust
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
```

`main.rs`/`test_server.rs` are their own crate roots so the attributes are
crate-scoped there; the four lib files scope to their module. `lib.rs` itself
gets NO attribute (a crate-root inner attr would cover every module — that flip
is the campaign's final-ratchet phase); its 8 documented items are
regression-guarded only by review until then — state this in the PLAN.md entry.

- [ ] **Step 2: Prove the ratchet bites**

Temporarily delete one doc comment in `config.rs`, run `cargo clippy
--all-targets -- -D warnings` from `src/server/` → expect ERROR (the deny
fires). Restore the comment, re-run → clean. (Anti-drift proof: a green gate
that cannot fail proves nothing.)

- [ ] **Step 3: Full verification sweep**

From repo root, all green: `(cd src/server && cargo test && cargo fmt --all
--check && cargo clippy --all-targets -- -D warnings)`; `pnpm docs:build`
(rustdoc regenerates with the new docs; link check green); scoped counts for
all six files → 0.

- [ ] **Step 4: Docs-sync + skill gate**

- `docs/PLAN.md` campaign section: mark Sweep 1 (server-ops) complete with the
  doctest-policy line ("calibration patterns live in the sweep-1 plan"), note
  the lib.rs deferred-deny caveat.
- `.claude/skills/shadowcat-codebase-server-ops/SKILL.md`: add a Gotcha — the
  six files carry `#![deny(missing_docs)]` + private-items twin; new items in
  them MUST ship documented or 3-OS clippy goes red; doctest policy pointer to
  this plan.
- Dispatch `shadowcat-spec-reviewer` on the skill diff (findings in final
  message; read-only git only). Fix findings.

- [ ] **Step 5: Merge + push + CI watch**

```bash
git checkout main && git merge --ff-only docs-sweep1-server-ops
git push origin main
gh run watch <run-id> --exit-status
```

All 7 CI jobs green required (the 3-OS clippy step now enforces the cluster's
docs). Fix-forward on red, topmost error first. Delete the merged branch.

---

## Deferred (logged, not dropped)

- Sweeps 2+ (next: server `data/` — 75+37+22+20+25+32+39+76 ≈ 300+ items across
  document.rs/sqlite.rs/command.rs/mod.rs/engine/*; likely split into 2–3
  plans), then ws/, http/+auth/, scene/, chat/+dice/; then client packages;
  then modules. Server-wide informational count at sweep-1 start: 1,059.
- lib.rs deny coverage: final-ratchet phase (crate-root attributes).
