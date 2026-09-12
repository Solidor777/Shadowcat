# M28 · Sandboxed third-party validators — Implementation Plan

**Goal:** opt-in, per-world, per-module-declared server-side validators running third-party
`wasm32-unknown-unknown` code inside `wasmi` (fuel/memory/instance-capped, no imports beyond a
rate-limited `env.log`), invoked exactly once per touched-`system`-band document (Create/Update,
recursing into embedded children by their own `doc_type`) at the `apply_intent`/`import_world`
chokepoint, over the `system` band only, after the engine's own validation — the pre-transaction
check first re-runs Phase 1's own pure structural validators on the pre-image so a malformed
submission never reaches — or counts against — a validator — never the default path, never able
to mutate/read/network, auto-disabled after 5 consecutive faults.

**Architecture:** `src/server/src/sandbox/` (new crate module: `runtime` — the wasmi host,
`registry` — compiled-module cache AND the per-(world, module) consecutive-fault counter) is
consulted by `SqliteRepository::apply_intent` (pre-write, outside the write transaction, against a
read-only pre-image) and `SqliteRepository::import_world` (inside its own already-exclusive
transaction). A refusal/fault surfaces through a new `DataError::Validator(ValidatorFault)`
variant and the existing `DataError::OpFailed`, mapped by `ws::conn`'s `reject_reason` to a NEW
`ServerMsg::Reject.detail: Option<String>` wire field the client renders as a toast. The
consecutive-fault counter is COUNTED entirely inside `sandbox::validate_document` (an in-memory,
restart-resettable `DashMap<(Uuid, String), u32>` living on `sandbox::registry::ValidatorRegistry`,
handed through unchanged across a rescan by `ValidatorRegistryCache`'s own persistent copy) and
ACTED ON by `ws::conn`: when a fault's `consecutive` count reaches `sandbox::VALIDATOR_FAULT_LIMIT`
(5), it calls `Room::disable_faulting_validator`, which disables the module for the world, posts a
GM-only notice, and resets that module's streak via the new `Repository::reset_validator_fault_streak`
trait method. Per-world opt-in moves the enabled-modules record from `Vec<String>` to
`Vec<WorldModuleEntry>`.

**Tech stack:** Rust (`wasmi`, `wat` dev-dep), TypeScript/Svelte 5 (client wiring), a standalone
`no_std` Rust→`wasm32-unknown-unknown` example crate outside the Cargo workspace.

**Spec:** `docs/superpowers/specs/2026-09-11-m28-sandboxed-validators-design.md` (THE spec —
read §1–§7 in full before touching code) plus
`docs/superpowers/specs/2026-09-11-phase3-master-integration-design.md` §0 (campaign directives),
§2.6 (the seam this milestone owns), §3 (shared-file conventions), §4 (global constraints), §5
(merge order — M28 merges SECOND, right after M22), §6 (skill-update gate), §7 (D8, the `wasmi`
ruling), §9 D8 (fault policy decision).

**Worktree:** `C:/Dev/Shadowcat-m28`, branch `m28-sandbox`.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

**Reporting rule:** a subagent delivers its report as the Agent tool result, via `SendMessage` to
the dispatcher, or by writing a named file; the dispatching prompt states which. An agent given a
`name` never returns a result — omit `name` for every dispatch whose report the dispatcher needs.

**Opus is banned** for every dispatch in this campaign. Coders are
`shadowcat-codebase:shadowcat-coder` (sonnet, effort medium); reviewers are
`shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`
(sonnet, effort high). Escalation goes to the `-fable` twins, never the `-opus` twins. Never end a
turn to ask whether to continue — run until the task list is exhausted or genuinely blocked.

## Model/Effort directives

- Implementation tasks → `shadowcat-codebase:shadowcat-coder` (sonnet, `effort: medium`).
- Buddy-check / final-diff review → BOTH `shadowcat-codebase:shadowcat-spec-reviewer` and
  `shadowcat-codebase:shadowcat-code-reviewer` (sonnet, `effort: high`), dispatched together,
  blind, against a dispatcher-pre-generated diff (reviewers have no Bash).
- Escalation on a BLOCKED report or a shallow/uncertain review: re-dispatch to the `-fable` twin
  of the same agent (`shadowcat-codebase:shadowcat-coder-fable`,
  `shadowcat-codebase:shadowcat-spec-reviewer-fable`,
  `shadowcat-codebase:shadowcat-code-reviewer-fable`) — never the `-opus` twin, never straight to
  the human, until the `-fable` twin also reports BLOCKED/shallow.

## Buddy-check directives

After EVERY task's gate battery passes and the commit lands, the dispatcher pre-generates the
task's diff (`git diff <parent>..HEAD -- <paths>`) and dispatches the spec+code reviewer pair
against it, blind (no shared context between the two, no access to the coder's own reasoning).
A finding either reviewer raises is fixed by the coder in a NEW commit before the next task starts
— never deferred, never silently overridden by the dispatcher. A finding the dispatcher disagrees
with is a design question for the user, not a unilateral override.

## Global constraints (verbatim from master §4)

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`);
  `pnpm lint:allowances` is a gate. Fix the code.
- File-size: 5,000-line soft limit needs the owner's allowlist signature, 10,000 hard; Rust
  test bodies in sibling files (`pnpm lint:file-size`, `pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, dates, sweep markers or history
  narration in `.ts`/`.rs`/`.svelte` (`pnpm lint:comments`).
- Every new `.ts` unit test that never touches the DOM opens with `// @vitest-environment node`.
- Deletion only through `trash`; never `rm`/`Remove-Item`/`git rm` as the sole step.
- Commits name their paths: `git commit -m "..." -- <paths>`; never `git add -A`.
- Long commands (`cargo test --all`, `pnpm -r test`, `pnpm build:all`) run in the background
  with output to a log file; read the log before claiming green.
- **Cross-platform:** `std::path` only; `#[cfg]`-gated OS code has an implementation for every
  target the matrix builds (Linux, macOS, Windows); responsive + touch-sized UI.
- **Licenses:** MIT / Apache-2.0 / BSD / zlib / MPL-2.0 only; media codecs royalty-free. Every
  new dependency lands with a `Cargo.toml`/`package.json` comment naming its license.
- **Binary size:** `pnpm lint:binary-size` guards the 60 MiB release binary; M28's runtime
  choice (wasmi, §7 of the master spec) is made under it.
- **Server by default:** computation runs on the server; client-side work needs a reason
  (presentation, input capture, optimistic prediction). `docs/design/ARCHITECTURE.md` §2
  invariants 1, 6 and 11 govern every design fork.
- **UX outranks data secrecy** (invariant 11): send-then-hide is acceptable; PII and
  remote-device security are the two ironclad exceptions.
- `pnpm build` precedes any cargo build (rust-embed validates `dist/` at compile time).
- The Playwright suite is DISPATCHER-run on port 31999 (one suite at a time on the machine);
  this milestone has no browser-driven feature (server-only), so no new spec is written.

**Full gate battery** (every task's Step 2/3 runs the subset that applies; the LAST task before
merge runs ALL of it): `cargo test --all`, `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings`, `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs
-D clippy::missing-docs-in-private-items`, `git diff --exit-code src/types/generated` after regen,
`pnpm -r typecheck`, `pnpm -r test`, `pnpm build`, `pnpm lint`, `lint:docs`, `lint:props`,
`lint:comments`, `lint:allowances`, `lint:file-size`, `lint:inline-tests`, `lint:aria-labels`,
`lint:gate-manifest`, `lint:settings-privacy`, `lint:binary-size` (release build),
`pnpm docs:check-examples`, `pnpm docs:check-rust-examples`, `pnpm run test:scripts`,
`pnpm run check:svelte-runtime`, `pnpm --filter "shadowcat-example-*" build`,
`pnpm --filter @shadowcat/core test:e2e`, `pnpm gate:push` immediately before `git push`.

## Spec-specified wall-clock pair (no longer ambiguous)

The spec's §3 now names two DISTINCT, explicitly-labeled wall-clock mechanisms rather than two
numbers that could be read as describing the same knob: a 50 ms post-hoc `Instant`-measured
duration on a call that DID return (reclassified `Fault(TooSlow)`), and a 250 ms
`tokio::time::timeout` around the `spawn_blocking` join for a call that never returns at all
(`Fault(Hung)`, a `FaultKind` variant distinct from `TooSlow`). Task 3 implements both as
parameterized budgets (`run_validator_with_budgets`) so `runtime::tests` can shrink the hang guard
far below the wall-clock cost of exhausting fuel, proving the timeout path fires for real rather
than merely observing a fault fuel exhaustion would have produced anyway.

## Design decisions this revision makes (recorded, not silently assumed)

- **Where the fault counter lives.** The spec's D8 ruling names the counter as living "on the
  sandbox registry the repository already holds." This plan reads that as
  `sandbox::registry::ValidatorRegistry` (with the underlying `DashMap` actually owned by
  `ValidatorRegistryCache`, so it survives a rescan that rebuilds the compiled-module map) rather
  than a field on `Room` — a compiled-module registry is rebuilt wholesale on every rescan, so
  the counter must be handed through by a shared `Arc`, not embedded as a by-value field that
  rebuild would silently zero.
- **`ValidatorFault` is the single payload type**, used identically by both
  `ValidatorVerdict::Fault(ValidatorFault)` and `DataError::Validator(ValidatorFault)` — the spec
  describes both with the same three fields; sharing one type is the DRY reading of that
  description rather than maintaining two independent field lists that could drift apart.
- **A module's own `Refuse` also resets its streak**, not only `Accept`. `Refuse` is an authored,
  WORKING decision — the module ran correctly and declined the write — so it is not evidence the
  module is technically broken. Only `Fault` (a trap, timeout, or ABI violation) is evidence of
  the thing `VALIDATOR_FAULT_LIMIT` exists to guard against.

Each of these is a spec gap the amendment left implicit, not a re-interpretation of an explicit
instruction; each is repeated in the HISTORY entry (Task 12) and in the final report.

---

### Task 1: `wasmi` + `wat` dependencies; correct the spec's API names against the resolved crate

**This is the ONE task allowed to edit the spec file**, per the spec's own §5 mandate: every wasmi
API name elsewhere in this document is unverified until this task runs.

**Files:**
- Modify: `src/server/Cargo.toml` — add, after the `regex = "1"` line and its preceding comment
  block, under a fresh header:
  ```toml

  # Sandboxed third-party validators (opt-in, per-world, over the `system` band only). wasmi
  # is a pure-Rust WASM interpreter: no JIT attack surface, no host imports beyond what
  # `sandbox::runtime` wires up, fuel-metered, and small (~1 MiB) — the only WASM runtime whose
  # own footprint fits the 60 MiB binary-size budget without adding a JIT or native codegen path.
  wasmi = "0.51"
  ```
  and, in `[dev-dependencies]`, after `serde_json = "1"`:
  ```toml
  # WAT-authored WASM test fixtures for `sandbox::runtime`'s unit tests (MIT/Apache-2.0).
  wat = "1"
  ```
- Modify: `docs/superpowers/specs/2026-09-11-m28-sandboxed-validators-design.md` §3 — after
  building against the resolved crate (Step 2 below), replace every wasmi API name/type/method
  this task found to differ from the crate's actual public API (`StoreLimits`/`StoreLimitsBuilder`,
  the fuel-enable/set/get method names, `Config`/`Engine`/`Store`/`Linker`/`Module`/`Instance`
  construction). Record the corrected names inline in §3's prose exactly where the old name
  appeared; do not restate the whole section.

- [ ] **Step 1:** `pnpm install` is NOT needed (Rust-only change). The worktree's `dist/` was
  already produced by the bootstrap that created `C:/Dev/Shadowcat-m28` and `rust-embed` validates
  it at Rust compile time, so `pnpm build` must precede any cargo build made after a client-side
  change anywhere in this plan — this task's own change is Rust-only, so no rebuild of `dist/` is
  needed here. Add the two dependency lines above; run `cargo build --manifest-path
  src/server/Cargo.toml` in the background, log to `debug/dumps/m28-cargo-build.log`; read the log
  to confirm the resolve succeeds and note the exact resolved `wasmi`/`wat` versions from
  `Cargo.lock`.
- [ ] **Step 2:** Generate the resolved crate's own docs: `cargo doc --manifest-path
  src/server/Cargo.toml -p wasmi --no-deps` (background, log to
  `debug/dumps/m28-wasmi-doc.log`) and open `target/doc/wasmi/index.html`'s generated HTML (or
  read the vendored source under `~/.cargo/registry/src/*/wasmi-<version>/src/`) to find the
  EXACT public names for: the typed trap-code accessor and its variants (`wasmi::Error::
  as_trap_code`, `wasmi::core::TrapCode::{OutOfFuel, MemoryOutOfBounds, TableOutOfBounds}` —
  `classify_trap` MUST use the typed code; matching a trap's `Display` text is forbidden because
  the wording is not API), the fuel-consumption toggle on `Config`, the store-side fuel
  set/get methods, the `StoreLimits`/`StoreLimitsBuilder` builder methods for memory size /
  instance count / table count, `Linker::func_wrap` (or equivalent) for defining `env.log`,
  `Instance`/`Store` typed-function-call and memory-access APIs. Write these down.
- [ ] **Step 3:** Apply the spec correction described in the Files section above. Every task
  below in THIS plan already uses the corrected names this step is expected to confirm
  (`Config::consume_fuel`, `Store::set_fuel`/`get_fuel`, `StoreLimitsBuilder::new().memory_size(..)
  .instances(..).tables(..).build()`, `Linker::new`, `Linker::func_wrap`, `Module::new`,
  `Instance::get_typed_func`, `Memory::data`/`data_mut`) — if the resolved crate's real names
  differ, correct BOTH the spec (this task) and every code snippet in Tasks 2–13 that used the
  wrong name, in the commit that introduces that code (fix-forward, per the crash-resolution
  directive — never leave a task's code referencing a name this task proved wrong).
- [ ] **Step 4:** `cargo fmt --check --manifest-path src/server/Cargo.toml`,
  `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings` PASS (no new
  code yet, so this only confirms the dependency addition itself compiles clean).
- [ ] **Step 5:** `git commit -m "build(server): add wasmi + wat dependencies; verify API names against the resolved crate" -- src/server/Cargo.toml src/server/Cargo.lock docs/superpowers/specs/2026-09-11-m28-sandboxed-validators-design.md`

---

### Task 2: `sandbox` module skeleton — verdict/fault types, guest ABI input shape

**Files:**
- Create: `src/server/src/sandbox/mod.rs`:
  ```rust
  //! Sandboxed third-party server-side validators: opt-in, per-world, per-module WASM code run
  //! inside `wasmi` over the `system` band only, after the engine's own validation. A validator
  //! can refuse a write (a structured reason) and nothing else — see `runtime` for the host,
  //! `registry` for the compiled-module cache. Every validator call is fuel/memory/instance-capped
  //! and has no host imports beyond a rate-limited `env.log`; `VALIDATOR_FAULT_LIMIT` bounds how
  //! many consecutive technical failures a module gets before a caller auto-disables it.
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]

  use serde::{Deserialize, Serialize};
  use uuid::Uuid;

  use crate::data::document::{Document, SchemaDeclaration};
  use crate::data::validation;
  use crate::data::DataError;

  pub mod registry;
  pub mod runtime;

  /// Technical failure of the sandbox itself — never a validator's own authored refusal
  /// (`ValidatorVerdict::Refuse`). Every variant maps to a `ValidatorFault`, whose `consecutive`
  /// count `ws::conn` compares against `VALIDATOR_FAULT_LIMIT` to decide whether to call
  /// `Room::disable_faulting_validator`.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum FaultKind {
      /// `consume_fuel` exhausted the per-call budget before `validate` returned.
      OutOfFuel,
      /// The guest exceeded its `StoreLimits` memory ceiling (e.g. an `alloc`/`memory.grow` bomb).
      MemoryLimit,
      /// The module is missing a required export, or an export has the wrong signature.
      BadAbi,
      /// `alloc` returned a pointer/length pair outside the guest's own linear memory.
      BadPointer,
      /// The serialized `ValidatorInput` exceeded 1 MiB; the module was never instantiated.
      InputTooLarge,
      /// The call RETURNED but its `Instant`-measured duration exceeded the 50 ms per-call
      /// budget — a slow-but-under-fuel validator, reclassified as a fault regardless of its
      /// own verdict.
      TooSlow,
      /// The call never returned at all within the 250 ms `tokio::time::timeout` hang guard
      /// around the `spawn_blocking` join (fuel bounds wasm instructions, not a stalled host
      /// import or allocator loop) — the blocking thread is abandoned, and this is logged at
      /// `warn`.
      Hung,
      /// Any other wasmi trap (unreachable, integer overflow, out-of-bounds table access, ...).
      Trap,
  }

  /// One sandboxed-validator technical failure: which module, what kind, and that module's
  /// current consecutive-fault streak for the world this call belongs to (including this
  /// fault). The SAME value both `ValidatorVerdict::Fault` and `DataError::Validator` carry —
  /// one type for "what technically went wrong," never duplicated across the two.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ValidatorFault {
      /// The faulting module's id.
      pub module: String,
      /// What went wrong.
      pub kind: FaultKind,
      /// This module's consecutive-fault streak for the world this call validated, INCLUDING
      /// this fault. Always `0` when the value has not yet been stamped by
      /// `validate_document` — `runtime::run_validator` itself has no world/registry context
      /// to compute the real streak.
      pub consecutive: u32,
  }

  /// Consecutive faults before a module's `validators_enabled` flag is auto-disabled for a
  /// world (the sandbox must never become a denial-of-service lever against the table).
  pub(crate) const VALIDATOR_FAULT_LIMIT: u32 = 5;

  /// What one validator call decided.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum ValidatorVerdict {
      /// The write may proceed as far as this validator is concerned.
      Accept,
      /// The write is refused with a player/GM-presentable reason (≤ 512 bytes, control
      /// characters stripped, lossy UTF-8 — see `runtime::run_validator`).
      Refuse {
          /// The refusing module's id.
          module: String,
          /// The reason text.
          reason: String,
      },
      /// The sandbox itself failed technically; the write is refused and the fault is counted.
      Fault(ValidatorFault),
  }

  /// The guest-visible input, UTF-8 JSON, passed by pointer/length through the guest's own
  /// `alloc` export. `#[serde(rename_all = "camelCase")]` produces the wire keys third-party
  /// guest code depends on; this struct's own field names stay idiomatic Rust — a field added
  /// here changes the wire contract every installed validator's guest code parses against.
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct ValidatorInput {
      /// The document's `doc_type`.
      pub doc_type: String,
      /// `"create"` or `"update"` (a `Move` op never touches `system`, so a validator never sees
      /// `"move"` in practice — the ABI reserves the value but nothing currently sends it).
      pub op: String,
      /// Post-image `system` band.
      pub system: serde_json::Value,
      /// Pre-image `system` band; `None` for a Create or a freshly-added embedded child.
      pub prior: Option<serde_json::Value>,
      /// The document's envelope `name`, if any.
      pub name: Option<String>,
      /// The world this write belongs to.
      pub world_id: Uuid,
      /// The validating module's own id (so a multi-`doc_type` validator can branch).
      pub module_id: String,
  }

  /// One document (or embedded descendant)'s validator input, gathered by `collect_validated_nodes`
  /// before any WASM runs — pure and synchronous, mirroring `validation::validate_system_schema_tree`'s
  /// recursion exactly (a child judged under its OWN `doc_type`; a child absent from `prior`
  /// validates with `system_prior: None`, i.e. as a Create).
  struct ValidatedNode<'a> {
      /// The node's `doc_type`.
      doc_type: &'a str,
      /// Post-image `system` band.
      system_post: &'a serde_json::Value,
      /// Pre-image `system` band, if this node existed before this write.
      system_prior: Option<&'a serde_json::Value>,
      /// The node's envelope `name`.
      name: Option<&'a str>,
  }

  /// Runs Phase 1's own pure structural validators against `doc` in place, in the EXACT order
  /// `SqliteRepository::apply_intent`'s Create and Update arms each run before their own Phase 2
  /// write: `validate_system_size`, `validate_property_overrides`, `validate_engine_tree`,
  /// `validate_system_size` again (the engine/note-derivation re-check both arms perform), then
  /// `validate_containment` and `validate_system_schema_tree` — every one of which already
  /// recurses `doc.embedded` on its own, so this runs once at the tree root, never per node.
  /// Returns Phase 1's own error on the first failure: a document that fails here would fail
  /// Phase 1 identically once it reaches the write transaction, so `validate_document` returns
  /// before consulting any validator — a malformed submission never reaches, and never faults, a
  /// validator. Phase 1 inside the transaction re-runs this exact chain, unchanged, and remains
  /// the sole authority over what actually commits.
  fn validate_structural(doc: &mut Document, schemas: &[SchemaDeclaration]) -> Result<(), DataError> {
      validation::validate_system_size(doc)?;
      validation::validate_property_overrides(doc)?;
      validation::validate_engine_tree(doc)?;
      validation::validate_system_size(doc)?;
      validation::validate_containment(doc)?;
      validation::validate_system_schema_tree(doc, schemas)?;
      Ok(())
  }

  /// Recurses `doc`'s embedded children exactly like `validation::validate_system_schema_tree`,
  /// pairing each with its PRIOR counterpart (same collection key, matched by id) when `prior`
  /// is `Some`.
  fn collect_validated_nodes<'a>(
      doc: &'a Document,
      prior: Option<&'a Document>,
      out: &mut Vec<ValidatedNode<'a>>,
  ) {
      out.push(ValidatedNode {
          doc_type: &doc.doc_type,
          system_post: &doc.system,
          system_prior: prior.map(|p| &p.system),
          name: doc.name.as_deref(),
      });
      for (key, children) in &doc.embedded {
          let prior_children = prior.and_then(|p| p.embedded.get(key));
          for child in children {
              let prior_child =
                  prior_children.and_then(|pc| pc.iter().find(|c| c.id == child.id));
              collect_validated_nodes(child, prior_child, out);
          }
      }
  }

  /// First runs `validate_structural` (Phase 1's own pure structural chain) against `doc`,
  /// returning its error untouched on failure without consulting any validator. Only a
  /// structurally valid `doc` reaches the wasm pass below: every matching enabled+opted-in
  /// validator, against `doc` and its embedded descendants, in ASCENDING module-id order —
  /// `enabled_module_ids` is sorted by this function itself, never trusted from the caller's own
  /// collection order, so `apply_intent` and `import_world` inherit identical, deterministic
  /// ordering with no way for the two chokepoints to fork — short-circuiting on the FIRST
  /// non-`Accept` verdict anywhere in the tree. `prior` is `doc`'s pre-image (`None` for a
  /// Create). Maintains `registry`'s per-(world, module) consecutive-fault counter itself: a
  /// module whose call faults has its streak incremented and stamped onto the returned
  /// `ValidatorVerdict::Fault`'s `consecutive` field; a module whose call returns `Accept` OR
  /// `Refuse` — either is a working, non-technical decision — has its own streak reset to zero; a
  /// module never consulted for this call (an earlier node/module short-circuited first, or the
  /// structural pre-pass rejected `doc` before any validator ran) is untouched either way.
  pub async fn validate_document(
      registry: &registry::ValidatorRegistry,
      enabled_module_ids: &[String],
      doc: &mut Document,
      prior: Option<&Document>,
      world_id: Uuid,
      schemas: &[SchemaDeclaration],
  ) -> Result<ValidatorVerdict, DataError> {
      validate_structural(doc, schemas)?;
      let mut ids: Vec<String> = enabled_module_ids.to_vec();
      ids.sort();
      let mut nodes = Vec::new();
      collect_validated_nodes(doc, prior, &mut nodes);
      for node in &nodes {
          for module_id in &ids {
              let Some(compiled) = registry.validator_for(module_id, node.doc_type) else {
                  continue;
              };
              let input = ValidatorInput {
                  doc_type: node.doc_type.to_string(),
                  op: if node.system_prior.is_some() {
                      "update".to_string()
                  } else {
                      "create".to_string()
                  },
                  system: node.system_post.clone(),
                  prior: node.system_prior.cloned(),
                  name: node.name.map(str::to_string),
                  world_id,
                  module_id: module_id.clone(),
              };
              match runtime::run_validator(compiled, &input).await {
                  ValidatorVerdict::Accept => {
                      registry.reset_faults(world_id, module_id);
                      continue;
                  }
                  ValidatorVerdict::Refuse { module, reason } => {
                      registry.reset_faults(world_id, &module);
                      return Ok(ValidatorVerdict::Refuse { module, reason });
                  }
                  ValidatorVerdict::Fault(fault) => {
                      let consecutive = registry.record_fault(world_id, &fault.module);
                      return Ok(ValidatorVerdict::Fault(ValidatorFault { consecutive, ..fault }));
                  }
              }
          }
      }
      Ok(ValidatorVerdict::Accept)
  }

  #[cfg(test)]
  mod tests;
  ```
- Create: `src/server/src/sandbox/registry.rs` (a minimal stub — Task 4 replaces `compiled`'s
  internal shape with the real filesystem scan, but `validator_for`/`record_fault`/
  `reset_faults`/`for_test`/`for_test_with_faults`'s signatures below are permanent):
  ```rust
  //! Compiles every enabled installed module's declared validators once, cached and
  //! invalidated the same way `crate::modules::ModuleScanCache` invalidates (mtime-keyed).
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]

  use std::collections::BTreeMap;
  use std::sync::Arc;

  use dashmap::DashMap;
  use uuid::Uuid;

  use super::runtime::CompiledValidator;

  /// Every installed module's compiled validator set, keyed by module id, then `doc_type`.
  #[derive(Default)]
  pub struct ValidatorRegistry {
      compiled: BTreeMap<String, BTreeMap<String, CompiledValidator>>,
      /// Per-(world, module) consecutive-fault counter, shared with every `ValidatorRegistry` a
      /// `ValidatorRegistryCache` hands out across a rescan — the counter must survive a
      /// rescan that only changes WHICH modules are compiled, never reset a fault streak in
      /// progress.
      faults: Arc<DashMap<(Uuid, String), u32>>,
  }

  impl ValidatorRegistry {
      /// The compiled validator `module_id` declares for `doc_type`, if any and if it compiled.
      pub fn validator_for(&self, module_id: &str, doc_type: &str) -> Option<&CompiledValidator> {
          self.compiled.get(module_id).and_then(|m| m.get(doc_type))
      }

      /// Records one consecutive sandbox fault for `(world, module)`, returning the new streak
      /// length. Maintained entirely by `sandbox::validate_document` — this registry never
      /// faults on its own.
      pub(crate) fn record_fault(&self, world: Uuid, module: &str) -> u32 {
          let mut entry = self.faults.entry((world, module.to_string())).or_insert(0);
          *entry += 1;
          *entry
      }

      /// Resets `(world, module)`'s consecutive-fault counter to zero — called on that module's
      /// own non-`Fault` verdict, and externally once a caller finishes disabling the module
      /// (so a future re-enable starts clean).
      pub(crate) fn reset_faults(&self, world: Uuid, module: &str) {
          self.faults.remove(&(world, module.to_string()));
      }
  }

  #[cfg(test)]
  impl ValidatorRegistry {
      /// Test-only constructor: a registry whose `validator_for` resolves exactly the given
      /// `(module_id, doc_type)` pairs to the given compiled validators, sharing a fresh
      /// fault-counter map.
      pub(crate) fn for_test(entries: Vec<(&str, &str, CompiledValidator)>) -> Self {
          Self::for_test_with_faults(entries, Arc::default())
      }

      /// As `for_test`, but sharing the given fault-counter map — lets a test drive two
      /// registries (e.g. a faulting one and an accepting one for the same module id) against
      /// the same underlying streak, mirroring how a rescan hands out a fresh
      /// `ValidatorRegistry` sharing the cache's one persistent counter.
      pub(crate) fn for_test_with_faults(
          entries: Vec<(&str, &str, CompiledValidator)>,
          faults: Arc<DashMap<(Uuid, String), u32>>,
      ) -> Self {
          let mut compiled: BTreeMap<String, BTreeMap<String, CompiledValidator>> = BTreeMap::new();
          for (module_id, doc_type, validator) in entries {
              compiled
                  .entry(module_id.to_string())
                  .or_default()
                  .insert(doc_type.to_string(), validator);
          }
          Self { compiled, faults }
      }
  }
  ```
- Create: `src/server/src/sandbox/tests.rs`:
  ```rust
  use super::*;
  use crate::data::document::{Document, PermissionSet, Scope};
  use dashmap::DashMap;
  use std::collections::BTreeMap;
  use std::sync::Arc;
  use wasmi::{Config, Engine, Module};

  fn doc(doc_type: &str, system: serde_json::Value) -> Document {
      Document {
          id: uuid::Uuid::new_v4(),
          scope: Scope::World { world_id: uuid::Uuid::nil() },
          doc_type: doc_type.into(),
          schema_version: 1,
          name: None,
          source: None,
          base: None,
          owner: None,
          permissions: PermissionSet::default(),
          embedded: BTreeMap::new(),
          parent_id: None,
          engine: None,
          system,
          created_at: 0,
          updated_at: 0,
      }
  }

  /// Compiles a minimal WAT fixture into a `CompiledValidator` for `module_id` — duplicated
  /// from `runtime::tests`' own equivalent helper, which is private to that module.
  fn compiled_for(module_id: &str, wat_src: &str) -> runtime::CompiledValidator {
      let mut config = Config::default();
      config.consume_fuel(true);
      let engine = Engine::new(&config);
      let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
      let module = Module::new(&engine, &bytes).expect("module compiles");
      runtime::CompiledValidator {
          module,
          module_id: module_id.to_string(),
      }
  }

  /// No `validate` export at all — every call faults with `FaultKind::BadAbi`, the cheapest
  /// fault to construct deterministically.
  const FAULTING_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0)))
  "#;

  /// Always accepts.
  const ACCEPTING_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
  "#;

  /// Always refuses with a fixed reason — an authored, WORKING decision, never a technical
  /// fault.
  const REFUSING_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (data (i32.const 2048) "no")
      (func (export "alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
      (func (export "reason_ptr") (result i32) (i32.const 2048))
      (func (export "reason_len") (result i32) (i32.const 2)))
  "#;

  #[test]
  fn collect_validated_nodes_recurses_embedded_children_paired_by_id() {
      let mut child_post = doc("combatant", serde_json::json!({ "hp": 5 }));
      let child_id = child_post.id;
      let mut parent_post = doc("combat", serde_json::json!({}));
      parent_post
          .embedded
          .insert("combatants".into(), vec![child_post.clone()]);

      let mut child_prior = child_post.clone();
      child_prior.system = serde_json::json!({ "hp": 10 });
      let mut parent_prior = parent_post.clone();
      parent_prior
          .embedded
          .insert("combatants".into(), vec![child_prior]);

      let mut nodes = Vec::new();
      collect_validated_nodes(&parent_post, Some(&parent_prior), &mut nodes);
      assert_eq!(nodes.len(), 2);
      assert_eq!(nodes[0].doc_type, "combat");
      assert_eq!(nodes[1].doc_type, "combatant");
      assert_eq!(nodes[1].system_prior, Some(&serde_json::json!({ "hp": 10 })));
      assert_eq!(nodes[1].system_post, &serde_json::json!({ "hp": 5 }));
      let _ = child_id;
  }

  #[test]
  fn collect_validated_nodes_treats_a_freshly_added_child_as_a_create() {
      let child = doc("combatant", serde_json::json!({ "hp": 5 }));
      let mut parent_post = doc("combat", serde_json::json!({}));
      parent_post
          .embedded
          .insert("combatants".into(), vec![child]);
      let parent_prior = doc("combat", serde_json::json!({}));

      let mut nodes = Vec::new();
      collect_validated_nodes(&parent_post, Some(&parent_prior), &mut nodes);
      assert_eq!(nodes[1].system_prior, None);
  }

  #[tokio::test]
  async fn validate_document_short_circuits_on_first_non_accept_in_module_id_order() {
      // No installed modules ⇒ the registry has nothing to match against ⇒ Accept.
      // (Full accept/refuse/fault coverage lives in `runtime::tests` and
      // `apply_intent`'s own integration tests — this module only owns the
      // tree-walk and short-circuit ordering.)
      let registry = registry::ValidatorRegistry::default();
      let mut d = doc("actor", serde_json::json!({ "hp": -1 }));
      let verdict = validate_document(&registry, &[], &mut d, None, uuid::Uuid::nil(), &[])
          .await
          .expect("structurally valid document");
      assert_eq!(verdict, ValidatorVerdict::Accept);
  }

  #[tokio::test]
  async fn per_module_fault_streaks_are_independent() {
      let registry = registry::ValidatorRegistry::for_test(vec![
          ("module-a", "actor", compiled_for("module-a", FAULTING_WAT)),
          ("module-b", "actor", compiled_for("module-b", ACCEPTING_WAT)),
      ]);
      let world = uuid::Uuid::from_u128(1);
      let a_only = vec!["module-a".to_string()];
      let b_only = vec!["module-b".to_string()];
      let mut d = doc("actor", serde_json::json!({}));

      for expected in 1..=4u32 {
          let verdict = validate_document(&registry, &a_only, &mut d, None, world, &[])
              .await
              .expect("structurally valid document");
          let ValidatorVerdict::Fault(fault) = verdict else {
              panic!("expected Fault, got {verdict:?}");
          };
          assert_eq!(fault.consecutive, expected);
      }

      // module-b's own accept, keyed on a DIFFERENT module id, never touches module-a's streak.
      let verdict = validate_document(&registry, &b_only, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      assert_eq!(verdict, ValidatorVerdict::Accept);

      let verdict = validate_document(&registry, &a_only, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      let ValidatorVerdict::Fault(fault) = verdict else {
          panic!("expected Fault, got {verdict:?}");
      };
      assert_eq!(fault.consecutive, 5, "module-b's accept must not have reset module-a's streak");
  }

  #[tokio::test]
  async fn a_modules_own_accept_resets_its_own_streak() {
      let faults: Arc<DashMap<(uuid::Uuid, String), u32>> = Arc::default();
      let faulting = registry::ValidatorRegistry::for_test_with_faults(
          vec![("module-a", "actor", compiled_for("module-a", FAULTING_WAT))],
          faults.clone(),
      );
      let accepting = registry::ValidatorRegistry::for_test_with_faults(
          vec![("module-a", "actor", compiled_for("module-a", ACCEPTING_WAT))],
          faults,
      );
      let world = uuid::Uuid::from_u128(2);
      let ids = vec!["module-a".to_string()];
      let mut d = doc("actor", serde_json::json!({}));

      for _ in 0..4 {
          validate_document(&faulting, &ids, &mut d, None, world, &[])
              .await
              .expect("structurally valid document");
      }

      // The SAME module's own accept — via a registry sharing the identical fault map, exactly
      // as a real rescan hands out a fresh `ValidatorRegistry` sharing the cache's one
      // persistent counter — resets module-a's streak to zero.
      let verdict = validate_document(&accepting, &ids, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      assert_eq!(verdict, ValidatorVerdict::Accept);

      let verdict = validate_document(&faulting, &ids, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      let ValidatorVerdict::Fault(fault) = verdict else {
          panic!("expected Fault, got {verdict:?}");
      };
      assert_eq!(fault.consecutive, 1, "module-a's own accept must have reset its streak to zero");
  }

  #[tokio::test]
  async fn a_modules_own_refuse_also_resets_its_streak() {
      // A `Refuse` is an authored, WORKING decision — the module ran correctly and declined
      // the write — so it resets the streak exactly like `Accept`; only `Fault` is evidence of
      // a technical break.
      let faults: Arc<DashMap<(uuid::Uuid, String), u32>> = Arc::default();
      let faulting = registry::ValidatorRegistry::for_test_with_faults(
          vec![("module-a", "actor", compiled_for("module-a", FAULTING_WAT))],
          faults.clone(),
      );
      let refusing = registry::ValidatorRegistry::for_test_with_faults(
          vec![("module-a", "actor", compiled_for("module-a", REFUSING_WAT))],
          faults,
      );
      let world = uuid::Uuid::from_u128(3);
      let ids = vec!["module-a".to_string()];
      let mut d = doc("actor", serde_json::json!({}));

      for _ in 0..4 {
          validate_document(&faulting, &ids, &mut d, None, world, &[])
              .await
              .expect("structurally valid document");
      }

      let verdict = validate_document(&refusing, &ids, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      assert!(matches!(verdict, ValidatorVerdict::Refuse { .. }));

      let verdict = validate_document(&faulting, &ids, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      let ValidatorVerdict::Fault(fault) = verdict else {
          panic!("expected Fault, got {verdict:?}");
      };
      assert_eq!(fault.consecutive, 1, "module-a's own refuse must have reset its streak to zero");
  }

  #[tokio::test]
  async fn enabled_module_order_is_sorted_by_validate_document_not_trusted_from_the_caller() {
      // Both modules ALWAYS refuse; requesting them in reverse-alphabetical order must still
      // yield module-a's own refusal, proving `validate_document` sorts `enabled_module_ids`
      // itself rather than trusting the caller's own collection order.
      let registry = registry::ValidatorRegistry::for_test(vec![
          ("module-b", "actor", compiled_for("module-b", REFUSING_WAT)),
          ("module-a", "actor", compiled_for("module-a", REFUSING_WAT)),
      ]);
      let world = uuid::Uuid::from_u128(5);
      let reverse_order = vec!["module-b".to_string(), "module-a".to_string()];
      let mut d = doc("actor", serde_json::json!({}));

      let verdict = validate_document(&registry, &reverse_order, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      let ValidatorVerdict::Refuse { module, .. } = verdict else {
          panic!("expected Refuse, got {verdict:?}");
      };
      assert_eq!(
          module, "module-a",
          "the alphabetically-first module's reason must win regardless of the caller's own enabled-list order"
      );
  }

  #[tokio::test]
  async fn a_structural_failure_never_reaches_or_faults_a_validator() {
      // `module-a`'s validator ALWAYS faults if it is ever consulted — this test's whole point
      // is that a structurally invalid `doc` never reaches it.
      let registry = registry::ValidatorRegistry::for_test(vec![(
          "module-a",
          "actor",
          compiled_for("module-a", FAULTING_WAT),
      )]);
      let world = uuid::Uuid::from_u128(4);
      let ids = vec!["module-a".to_string()];
      // A tier-2 schema requiring `/system/hp` to be a number; the document violates it.
      let schemas = vec![crate::data::document::SchemaDeclaration {
          module_id: "example-system".into(),
          version: "1".into(),
          schema_format: 1,
          doc_type: "actor".into(),
          subtree_pointer: "/system/hp".into(),
          schema: serde_json::from_value(serde_json::json!({ "type": "number" })).unwrap(),
      }];
      let mut d = doc("actor", serde_json::json!({ "hp": "not-a-number" }));

      let err = validate_document(&registry, &ids, &mut d, None, world, &schemas)
          .await
          .expect_err("a tier-2 schema violation must surface as Phase 1's own error");
      assert!(
          matches!(err, crate::data::DataError::SchemaViolation { .. }),
          "expected SchemaViolation, got {err:?}"
      );

      // `record_fault` increments from whatever is currently stored and returns the new total —
      // a fresh `1` here proves module-a's streak was still zero, i.e. the rejected document
      // above never reached (and never faulted) the validator.
      assert_eq!(
          registry.record_fault(world, "module-a"),
          1,
          "a structurally-invalid document must never have counted against a validator's fault streak"
      );
  }
  ```

- [ ] **Step 1:** write the test file first (it references `registry::ValidatorRegistry::default()`,
  `registry::ValidatorRegistry::for_test`/`for_test_with_faults`, and `runtime::run_validator`,
  which do not exist yet). This step creates `registry.rs` with real, permanent surface (Task 4
  fleshes out the real filesystem scan but does not change these signatures): `#[derive(Default)]
  pub struct ValidatorRegistry { compiled: std::collections::BTreeMap<String,
  std::collections::BTreeMap<String, runtime::CompiledValidator>>, faults:
  std::sync::Arc<dashmap::DashMap<(uuid::Uuid, String), u32>> }`; `pub fn validator_for(&self,
  module_id: &str, doc_type: &str) -> Option<&runtime::CompiledValidator>` returning
  `self.compiled.get(module_id).and_then(|m| m.get(doc_type))`; `pub(crate) fn record_fault(&self,
  world: Uuid, module: &str) -> u32` (increments and returns the new streak length via
  `self.faults.entry((world, module.to_string())).or_insert(0)`); `pub(crate) fn
  reset_faults(&self, world: Uuid, module: &str)` (`self.faults.remove(&(world,
  module.to_string()))`); and, `#[cfg(test)]`-gated, `pub(crate) fn for_test(entries)`/`pub(crate)
  fn for_test_with_faults(entries, faults)` test constructors (full code below).
- [ ] **Step 2:** add `pub mod sandbox;` to `src/server/src/lib.rs`, alphabetically between
  `pub mod modules;` and `pub mod scene;`; rewrite the crate doc comment's closing sentence at
  the top of the file from `Server-side code never executes third-party module code.` to
  `Server-side code never executes third-party module code, except opted-in sandboxed
  validators (` + "`sandbox`" + `) running inside a fuel/memory-limited ` + "`wasmi`" + ` interpreter
  with no host imports beyond a rate-limited debug log.`
- [ ] **Step 3:** `cargo test --all --manifest-path src/server/Cargo.toml` (background + log) PASS
  for the new `sandbox` tests; `cargo clippy --manifest-path src/server/Cargo.toml --all-targets
  -- -D warnings`, `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D
  missing-docs -D clippy::missing-docs-in-private-items`, `cargo fmt --check
  --manifest-path src/server/Cargo.toml` PASS.
- [ ] **Step 4:** `git commit -m "feat(sandbox): verdict/fault types, guest ABI input, embedded-tree walk, per-(world,module) fault counter" -- src/server/src/sandbox/ src/server/src/lib.rs`

---

### Task 3: `sandbox::runtime` — the wasmi host

**Files:**
- Create: `src/server/src/sandbox/runtime.rs`:
  ```rust
  //! The wasmi host: compiles a validator module once (`CompiledValidator`, held by
  //! `super::registry::ValidatorRegistry`) and runs it per call on a fresh `Store` with hard
  //! fuel/memory/instance/table limits and no imports beyond `env.log`.
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]

  use std::time::{Duration, Instant};

  use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

  use super::{FaultKind, ValidatorFault, ValidatorInput, ValidatorVerdict};

  /// Input JSON above this size is refused without ever instantiating the module.
  const MAX_INPUT_BYTES: usize = 1024 * 1024;
  /// The refusal reason text is truncated (byte-safe UTF-8 boundary) to this length.
  const MAX_REASON_BYTES: usize = 512;
  /// Fuel budget per call — `wasmi`'s fuel unit is roughly one interpreted instruction, so this
  /// bounds a validator to tens of millions of instructions regardless of any loop it authors.
  const MAX_FUEL: u64 = 50_000_000;
  /// Guest linear memory ceiling.
  const MAX_MEMORY_BYTES: usize = 16 * 1024 * 1024;
  /// Post-hoc wall-clock threshold: a call that RETURNED but took longer than this is
  /// reclassified as `Fault(FaultKind::TooSlow)` regardless of its own verdict.
  const SLOW_CALL_BUDGET: Duration = Duration::from_millis(50);
  /// Hard hang guard around the whole `spawn_blocking` join: a call that never returns within
  /// this window is abandoned and reported as `Fault(FaultKind::Hung)` — distinct from
  /// `TooSlow`, which is a call that DID return (fuel bounds wasm instructions, not a stalled
  /// host import or allocator loop, so this is a belt-and-braces guard against the latter).
  const HANG_GUARD: Duration = Duration::from_millis(250);
  /// Per-call cap on `env.log` payload size.
  const MAX_LOG_BYTES: usize = 1024;
  /// Per-call cap on `env.log` invocation count; the 17th+ call is silently ignored.
  const MAX_LOG_CALLS: u32 = 16;

  /// One compiled, cached validator: the wasmi `Module` plus the declaring module/doc_type
  /// this validator belongs to (for fault reporting).
  #[derive(Clone)]
  pub struct CompiledValidator {
      /// The compiled wasmi module.
      pub(super) module: Module,
      /// The declaring installed-module id (for `ValidatorVerdict`'s `module` field).
      pub(super) module_id: String,
  }

  /// Per-call host state: the guest's memory limiter and the `env.log` call/byte budget.
  struct HostState {
      /// Enforces `MAX_MEMORY_BYTES`/1 instance/1 table.
      limits: StoreLimits,
      /// Remaining `env.log` calls this store may still honour.
      log_calls_remaining: u32,
  }

  /// Runs `compiled` against `input` with the production wall-clock budgets
  /// (`SLOW_CALL_BUDGET`/`HANG_GUARD`) — see `run_validator_with_budgets` for the
  /// implementation and why the two are parameters rather than the module consts directly.
  pub async fn run_validator(compiled: &CompiledValidator, input: &ValidatorInput) -> ValidatorVerdict {
      run_validator_with_budgets(compiled, input, SLOW_CALL_BUDGET, HANG_GUARD).await
  }

  /// Runs `compiled` against `input` on `tokio::task::spawn_blocking`, on a fresh `Store` per
  /// call. Every trap, out-of-fuel, missing export, out-of-bounds pointer or non-UTF-8 reason
  /// (lossy-decoded, never a fault) maps to a `ValidatorVerdict::Fault`; a `validate` return of
  /// `0` is `Accept`; any other return reads the reason via `reason_ptr`/`reason_len`. Every
  /// `Fault` this function produces carries `consecutive: 0` — this function has no world or
  /// registry context to compute the real streak; `sandbox::validate_document` stamps the real
  /// value in before returning the verdict to ITS OWN caller. `slow_call_budget`/`hang_guard`
  /// are parameters (not the module consts directly) so `runtime::tests` can shrink
  /// `hang_guard` far below the wall-clock cost of exhausting `MAX_FUEL`, proving the guard's
  /// own `tokio::time::timeout` fires for real rather than merely observing a fault fuel
  /// exhaustion would have produced anyway.
  async fn run_validator_with_budgets(
      compiled: &CompiledValidator,
      input: &ValidatorInput,
      slow_call_budget: Duration,
      hang_guard: Duration,
  ) -> ValidatorVerdict {
      let module_id = compiled.module_id.clone();
      let fault = |kind: FaultKind| {
          ValidatorVerdict::Fault(ValidatorFault {
              module: module_id.clone(),
              kind,
              consecutive: 0,
          })
      };
      let bytes = match serde_json::to_vec(input) {
          Ok(b) => b,
          Err(_) => return fault(FaultKind::BadAbi),
      };
      if bytes.len() > MAX_INPUT_BYTES {
          return fault(FaultKind::InputTooLarge);
      }
      let compiled = compiled.clone();
      let started = Instant::now();
      let join = tokio::task::spawn_blocking(move || run_validator_blocking(&compiled, &bytes));
      let result = match tokio::time::timeout(hang_guard, join).await {
          Ok(Ok(verdict)) => verdict,
          Ok(Err(_)) => fault(FaultKind::Trap),
          Err(_) => {
              tracing::warn!(module = %module_id, "validator call exceeded the hang guard");
              return fault(FaultKind::Hung);
          }
      };
      let elapsed = started.elapsed();
      if elapsed > slow_call_budget {
          tracing::warn!(module = %module_id, ?elapsed, "validator call exceeded the slow-call budget");
          return fault(FaultKind::TooSlow);
      }
      result
  }

  /// The synchronous body run inside `spawn_blocking`: fresh `Engine`/`Store`/`Linker`,
  /// instantiate, `alloc` → write input → `validate` → read reason.
  fn run_validator_blocking(compiled: &CompiledValidator, input_bytes: &[u8]) -> ValidatorVerdict {
      let module_id = compiled.module_id.clone();
      let fault = |kind: FaultKind| {
          ValidatorVerdict::Fault(ValidatorFault {
              module: module_id.clone(),
              kind,
              consecutive: 0,
          })
      };

      let mut config = Config::default();
      config.consume_fuel(true);
      let engine = Engine::new(&config);

      let limits = StoreLimitsBuilder::new()
          .memory_size(MAX_MEMORY_BYTES)
          .instances(1)
          .tables(1)
          .build();
      let mut store = Store::new(
          &engine,
          HostState {
              limits,
              log_calls_remaining: MAX_LOG_CALLS,
          },
      );
      store.limiter(|state| &mut state.limits);
      if store.set_fuel(MAX_FUEL).is_err() {
          return fault(FaultKind::Trap);
      }

      let mut linker: Linker<HostState> = Linker::new(&engine);
      if linker
          .func_wrap(
              "env",
              "log",
              |mut caller: wasmi::Caller<'_, HostState>, ptr: i32, len: i32| {
                  if caller.data().log_calls_remaining == 0 {
                      return;
                  }
                  let len = (len as usize).min(MAX_LOG_BYTES);
                  let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                  else {
                      return;
                  };
                  let ptr = ptr as usize;
                  if let Some(bytes) = memory.data(&caller).get(ptr..ptr + len) {
                      let text = String::from_utf8_lossy(bytes);
                      tracing::debug!(target: "sandbox::guest_log", %text);
                  }
                  caller.data_mut().log_calls_remaining -= 1;
              },
          )
          .is_err()
      {
          return fault(FaultKind::BadAbi);
      }

      let instance = match linker.instantiate(&mut store, &compiled.module) {
          Ok(pre) => match pre.start(&mut store) {
              Ok(i) => i,
              Err(_) => return fault(FaultKind::Trap),
          },
          Err(_) => return fault(FaultKind::BadAbi),
      };

      let Ok(memory) = instance
          .get_export(&store, "memory")
          .and_then(|e| e.into_memory())
          .ok_or(())
      else {
          return fault(FaultKind::BadAbi);
      };
      let Ok(alloc) = instance.get_typed_func::<i32, i32>(&store, "alloc") else {
          return fault(FaultKind::BadAbi);
      };
      let Ok(validate) = instance.get_typed_func::<(i32, i32), i32>(&store, "validate") else {
          return fault(FaultKind::BadAbi);
      };

      let ptr = match alloc.call(&mut store, input_bytes.len() as i32) {
          Ok(p) if p >= 0 => p as usize,
          Ok(_) => return fault(FaultKind::BadPointer),
          Err(e) => return fault(classify_trap(&e)),
      };
      {
          let mem = memory.data_mut(&mut store);
          let Some(slot) = mem.get_mut(ptr..ptr + input_bytes.len()) else {
              return fault(FaultKind::BadPointer);
          };
          slot.copy_from_slice(input_bytes);
      }

      let result = match validate.call(&mut store, (ptr as i32, input_bytes.len() as i32)) {
          Ok(r) => r,
          Err(e) => return fault(classify_trap(&e)),
      };
      if result == 0 {
          return ValidatorVerdict::Accept;
      }

      let reason = read_reason(&instance, &mut store, &memory).unwrap_or_else(|kind| return_early(kind));
      match reason {
          Ok(text) => ValidatorVerdict::Refuse {
              module: module_id,
              reason: text,
          },
          Err(kind) => fault(kind),
      }
  }

  /// Reads the guest's `reason_ptr()`/`reason_len()` exports and the UTF-8 (lossy) text at that
  /// range, truncated to `MAX_REASON_BYTES` with control characters stripped.
  fn read_reason(
      instance: &wasmi::Instance,
      store: &mut Store<HostState>,
      memory: &wasmi::Memory,
  ) -> Result<Result<String, FaultKind>, FaultKind> {
      let Ok(reason_ptr) = instance.get_typed_func::<(), i32>(&*store, "reason_ptr") else {
          return Err(FaultKind::BadAbi);
      };
      let Ok(reason_len) = instance.get_typed_func::<(), i32>(&*store, "reason_len") else {
          return Err(FaultKind::BadAbi);
      };
      let ptr = match reason_ptr.call(&mut *store, ()) {
          Ok(p) if p >= 0 => p as usize,
          _ => return Err(FaultKind::BadPointer),
      };
      let len = match reason_len.call(&mut *store, ()) {
          Ok(l) if l >= 0 => (l as usize).min(MAX_REASON_BYTES),
          _ => return Err(FaultKind::BadPointer),
      };
      let Some(bytes) = memory.data(&*store).get(ptr..ptr + len) else {
          return Err(FaultKind::BadPointer);
      };
      let text: String = String::from_utf8_lossy(bytes)
          .chars()
          .filter(|c| !c.is_control())
          .collect();
      Ok(Ok(text))
  }

  /// Placeholder never actually reached — `read_reason`'s `Result<Result<..>>` always returns
  /// `Ok(..)`; this only exists to keep the `unwrap_or_else` call site total over the type.
  fn return_early(kind: FaultKind) -> Result<String, FaultKind> {
      Err(kind)
  }

  /// wasmi trap classification through the TYPED trap code, never the error's `Display` text:
  /// `wasmi::Error::as_trap_code` yields the `TrapCode` a trap carries (`OutOfFuel` when the
  /// store's fuel is exhausted, `MemoryOutOfBounds`/`TableOutOfBounds` when the guest addresses
  /// past its current memory). A `memory.grow` the `StoreLimits` ceiling denies never traps —
  /// core Wasm returns -1 to the guest — so the ceiling itself is invisible here; it becomes a
  /// `MemoryLimit` fault only when the guest then touches memory it failed to obtain. Any other
  /// trap code, or an error carrying no trap code (a host-import error), is a generic `Trap`.
  fn classify_trap(e: &wasmi::Error) -> FaultKind {
      use wasmi::core::TrapCode;
      match e.as_trap_code() {
          Some(TrapCode::OutOfFuel) => FaultKind::OutOfFuel,
          Some(TrapCode::MemoryOutOfBounds | TrapCode::TableOutOfBounds) => FaultKind::MemoryLimit,
          _ => FaultKind::Trap,
      }
  }

  #[cfg(test)]
  mod tests;
  ```
- Create: `src/server/src/sandbox/runtime/tests.rs` — one WAT module per accept/refuse/fault
  case (spec §6), built with the `wat` crate at test time:
  ```rust
  use super::*;
  use crate::sandbox::{ValidatorFault, ValidatorInput};

  fn compiled(wat_src: &str) -> CompiledValidator {
      let mut config = Config::default();
      config.consume_fuel(true);
      let engine = Engine::new(&config);
      let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
      let module = Module::new(&engine, &bytes).expect("module compiles");
      CompiledValidator {
          module,
          module_id: "test-module".into(),
      }
  }

  fn input(hp: i64) -> ValidatorInput {
      ValidatorInput {
          doc_type: "actor".into(),
          op: "create".into(),
          system: serde_json::json!({ "hp": hp }),
          prior: None,
          name: None,
          world_id: uuid::Uuid::nil(),
          module_id: "test-module".into(),
      }
  }

  /// A validator that always accepts: `alloc` bump-allocates from a static offset,
  /// `validate` always returns 0.
  const ACCEPT_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (global $next (mut i32) (i32.const 1024))
      (func (export "alloc") (param $len i32) (result i32)
        (local $p i32)
        global.get $next
        local.set $p
        global.get $next
        local.get $len
        i32.add
        global.set $next
        local.get $p)
      (func (export "validate") (param i32 i32) (result i32)
        (i32.const 0)))
  "#;

  #[tokio::test]
  async fn accept_returns_zero() {
      let verdict = run_validator(&compiled(ACCEPT_WAT), &input(10)).await;
      assert_eq!(verdict, ValidatorVerdict::Accept);
  }

  /// Refuses unconditionally with a fixed reason string stored at a static offset.
  const REFUSE_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (data (i32.const 2048) "negative hp")
      (global $next (mut i32) (i32.const 1024))
      (func (export "alloc") (param $len i32) (result i32)
        (local $p i32)
        global.get $next
        local.set $p
        global.get $next
        local.get $len
        i32.add
        global.set $next
        local.get $p)
      (func (export "validate") (param i32 i32) (result i32)
        (i32.const 1))
      (func (export "reason_ptr") (result i32) (i32.const 2048))
      (func (export "reason_len") (result i32) (i32.const 11)))
  "#;

  #[tokio::test]
  async fn refuse_with_reason() {
      let verdict = run_validator(&compiled(REFUSE_WAT), &input(-1)).await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Refuse {
              module: "test-module".into(),
              reason: "negative hp".into(),
          }
      );
  }

  /// Same as `REFUSE_WAT` but `reason_len` claims far more than `MAX_REASON_BYTES`; the host
  /// truncates rather than reading out of bounds or refusing outright.
  const REFUSE_LONG_REASON_WAT: &str = r#"
    (module
      (memory (export "memory") 20)
      (data (i32.const 2048) "x")
      (global $next (mut i32) (i32.const 4096))
      (func (export "alloc") (param $len i32) (result i32)
        (local $p i32)
        global.get $next
        local.set $p
        global.get $next
        local.get $len
        i32.add
        global.set $next
        local.get $p)
      (func (export "validate") (param i32 i32) (result i32)
        (i32.const 1))
      (func (export "reason_ptr") (result i32) (i32.const 2048))
      (func (export "reason_len") (result i32) (i32.const 1048576)))
  "#;

  #[tokio::test]
  async fn refuse_with_an_over_long_reason_is_truncated() {
      let verdict = run_validator(&compiled(REFUSE_LONG_REASON_WAT), &input(-1)).await;
      let ValidatorVerdict::Refuse { reason, .. } = verdict else {
          panic!("expected Refuse, got {verdict:?}");
      };
      assert!(reason.len() <= 512);
  }

  /// An infinite loop burns fuel until the interpreter traps — also reused, with a shrunk
  /// `hang_guard`, to prove the hang-guard timeout path fires before fuel exhaustion would.
  const INFINITE_LOOP_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "validate") (param i32 i32) (result i32)
        (loop $l (br $l))
        (i32.const 0)))
  "#;

  #[tokio::test]
  async fn infinite_loop_is_out_of_fuel() {
      let verdict = run_validator(&compiled(INFINITE_LOOP_WAT), &input(0)).await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::OutOfFuel,
              consecutive: 0,
          })
      );
  }

  /// Grows memory one page at a time until the `StoreLimits` ceiling denies the grow (which
  /// returns -1, never traps), then stores one byte past the last page it did obtain — the
  /// out-of-bounds access is what traps. Bounded: the ceiling is a few pages, so the loop ends
  /// long before the fuel budget does.
  const MEMORY_BOMB_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "validate") (param i32 i32) (result i32)
        (block $done
          (loop $l
            (br_if $done (i32.eq (memory.grow (i32.const 1)) (i32.const -1)))
            (br $l)))
        (i32.store (i32.mul (memory.size) (i32.const 65536)) (i32.const 1))
        (i32.const 0)))
  "#;

  #[tokio::test]
  async fn memory_bomb_is_a_memory_limit_fault() {
      // The denied grow is silent (-1); the store past the obtained pages is the trap, and it
      // must classify as MemoryLimit — never OutOfFuel, which would mean the ceiling was not
      // enforced and the loop ran until the fuel ran out.
      let verdict = run_validator(&compiled(MEMORY_BOMB_WAT), &input(0)).await;
      let ValidatorVerdict::Fault(ValidatorFault { kind, .. }) = verdict else {
          panic!("expected Fault, got {verdict:?}");
      };
      assert_eq!(kind, FaultKind::MemoryLimit);
  }

  /// No `validate` export at all.
  const MISSING_VALIDATE_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0)))
  "#;

  #[tokio::test]
  async fn missing_validate_export_is_bad_abi() {
      let verdict = run_validator(&compiled(MISSING_VALIDATE_WAT), &input(0)).await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::BadAbi,
              consecutive: 0,
          })
      );
  }

  /// `alloc` returns a pointer past the guest's own single-page memory.
  const BAD_POINTER_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 999999))
      (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
  "#;

  #[tokio::test]
  async fn alloc_out_of_bounds_is_bad_pointer() {
      let verdict = run_validator(&compiled(BAD_POINTER_WAT), &input(0)).await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::BadPointer,
              consecutive: 0,
          })
      );
  }

  /// The reason bytes are not valid UTF-8; `String::from_utf8_lossy` replaces them rather than
  /// faulting.
  const NON_UTF8_REASON_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (data (i32.const 2048) "\ff\fe")
      (func (export "alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
      (func (export "reason_ptr") (result i32) (i32.const 2048))
      (func (export "reason_len") (result i32) (i32.const 2)))
  "#;

  #[tokio::test]
  async fn non_utf8_reason_is_lossy_decoded_not_a_fault() {
      let verdict = run_validator(&compiled(NON_UTF8_REASON_WAT), &input(0)).await;
      assert!(matches!(verdict, ValidatorVerdict::Refuse { .. }));
  }

  /// Calls `env.log` 17 times; the host counts only 16.
  const LOG_SEVENTEEN_TIMES_WAT: &str = r#"
    (module
      (import "env" "log" (func $log (param i32 i32)))
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "validate") (param i32 i32) (result i32)
        (local $i i32)
        (local.set $i (i32.const 0))
        (block $done
          (loop $l
            (br_if $done (i32.ge_u (local.get $i) (i32.const 17)))
            (call $log (i32.const 0) (i32.const 1))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $l)))
        (i32.const 0)))
  "#;

  #[tokio::test]
  async fn sixteen_log_calls_honoured_seventeenth_ignored() {
      // No assertion on the log sink itself here (that's `tracing`'s own concern) — this test
      // pins that the 17th call does not trap or otherwise change the verdict.
      let verdict = run_validator(&compiled(LOG_SEVENTEEN_TIMES_WAT), &input(0)).await;
      assert_eq!(verdict, ValidatorVerdict::Accept);
  }

  #[tokio::test]
  async fn input_over_one_mib_is_refused_without_running() {
      let mut oversized = input(0);
      oversized.name = Some("x".repeat(2 * 1024 * 1024));
      let verdict = run_validator(&compiled(ACCEPT_WAT), &oversized).await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::InputTooLarge,
              consecutive: 0,
          })
      );
  }

  #[tokio::test]
  async fn a_call_that_never_returns_faults_hung_once_the_hang_guard_fires_first() {
      // `hang_guard` is shrunk far below the wall-clock cost of burning `MAX_FUEL`'s
      // 50_000_000 units in a tight interpreted loop, and `slow_call_budget` is generously
      // wide — so ONLY the hang guard can be what fires, proving the timeout path for real
      // rather than merely observing a fault fuel exhaustion would have produced anyway.
      let verdict = run_validator_with_budgets(
          &compiled(INFINITE_LOOP_WAT),
          &input(0),
          Duration::from_secs(3600),
          Duration::from_millis(1),
      )
      .await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::Hung,
              consecutive: 0,
          })
      );
  }

  #[tokio::test]
  async fn a_call_that_returns_past_the_slow_call_budget_is_reclassified_too_slow() {
      // A zero-duration budget is exceeded by any measurable call, deterministically
      // exercising the post-hoc reclassification without depending on real sleep timing the
      // guest ABI has no way to request.
      let verdict = run_validator_with_budgets(
          &compiled(ACCEPT_WAT),
          &input(0),
          Duration::ZERO,
          Duration::from_millis(250),
      )
      .await;
      assert_eq!(
          verdict,
          ValidatorVerdict::Fault(ValidatorFault {
              module: "test-module".into(),
              kind: FaultKind::TooSlow,
              consecutive: 0,
          })
      );
  }
  ```

- [ ] **Step 1:** the tests above are written FIRST against `run_validator`'s signature; then
  implement `runtime.rs`. Fix every wasmi API name against Task 1's corrected spec §3 as
  compile errors surface — this is expected fix-forward work, not a plan defect.
- [ ] **Step 2:** `cargo test --all --manifest-path src/server/Cargo.toml -- sandbox::runtime`
  (background + log) PASS; full `cargo test --all` (background + log) PASS; `cargo clippy
  --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`, the `missing-docs`
  clippy invocation, `cargo fmt --check` PASS.
- [ ] **Step 3:** `git commit -m "feat(sandbox): the wasmi host — fuel/memory/instance-limited per-call runtime" -- src/server/src/sandbox/runtime.rs src/server/src/sandbox/runtime/`

---

### Task 4: manifest `validators` key, `InstalledModule.validators`, traversal guard, registry scan

**Files:**
- Modify: `src/server/src/http/module_routes.rs` — change `fn is_strictly_within` from private
  to `pub(crate) fn is_strictly_within` (no other change to its body); this is the exact
  traversal guard `modules.rs`'s scan-time check reuses.
- Modify: `src/server/src/sandbox/mod.rs` — after the `runtime`/`registry` module declarations,
  no change needed here (registry owns the scan).
- Modify: `src/server/src/sandbox/registry.rs` — replace the Task 2 stub with:
  ```rust
  //! Compiles every enabled installed module's declared validators once, cached and
  //! invalidated the same way `crate::modules::ModuleScanCache` invalidates (mtime-keyed).
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]

  use std::collections::BTreeMap;
  use std::path::Path;
  use std::sync::{Arc, Mutex};

  use dashmap::DashMap;
  use uuid::Uuid;
  use wasmi::{Config, Engine, Module};

  use super::runtime::CompiledValidator;

  /// A module's declared validators fail to compile as a whole (one bad `.wasm` never blocks
  /// the others — each validator entry is independent) recorded per-module for `ModuleManager`'s
  /// load-status display.
  #[derive(Debug, Clone, Default)]
  pub struct ModuleLoadResult {
      /// `doc_type -> compiled validator`, only entries that compiled successfully.
      pub by_doc_type: BTreeMap<String, CompiledValidator>,
      /// The FIRST compile failure encountered for this module's declared validators, if any.
      pub load_error: Option<String>,
  }

  /// Every installed module's compiled validator set, keyed by module id, then `doc_type`.
  #[derive(Default)]
  pub struct ValidatorRegistry {
      by_module: BTreeMap<String, ModuleLoadResult>,
      /// Per-(world, module) consecutive-fault counter, shared with the `ValidatorRegistryCache`
      /// that produced this registry (and every other registry that cache has ever produced) —
      /// see `ValidatorRegistryCache::faults`'s own doc for why continuity survives a rescan.
      faults: Arc<DashMap<(Uuid, String), u32>>,
  }

  impl ValidatorRegistry {
      /// The compiled validator `module_id` declares for `doc_type`, if any and if it compiled.
      pub fn validator_for(&self, module_id: &str, doc_type: &str) -> Option<&CompiledValidator> {
          self.by_module.get(module_id)?.by_doc_type.get(doc_type)
      }

      /// This module's compile diagnostic, if its declared validators failed to load — shown by
      /// `GET /api/modules`'s `validator_load_error`.
      pub fn load_error_for(&self, module_id: &str) -> Option<&str> {
          self.by_module
              .get(module_id)
              .and_then(|r| r.load_error.as_deref())
      }

      /// Records one consecutive sandbox fault for `(world, module)`, returning the new streak
      /// length. Maintained entirely by `sandbox::validate_document` — this registry never
      /// faults on its own.
      pub(crate) fn record_fault(&self, world: Uuid, module: &str) -> u32 {
          let mut entry = self.faults.entry((world, module.to_string())).or_insert(0);
          *entry += 1;
          *entry
      }

      /// Resets `(world, module)`'s consecutive-fault counter to zero — called on that module's
      /// own non-`Fault` verdict, and externally (via `ValidatorRegistryCache::reset_faults`)
      /// once a caller finishes disabling the module (so a future re-enable starts clean).
      pub(crate) fn reset_faults(&self, world: Uuid, module: &str) {
          self.faults.remove(&(world, module.to_string()));
      }

      /// Compile every declared validator of every module `crate::modules::scan_installed_modules`
      /// found under `modules_dir`. A module whose `.wasm` fails to read or fails to compile as a
      /// valid WASM module gets `load_error: Some(..)` and NO entries — the module itself still
      /// scans/loads normally (fail-open discovery, the same posture `scan_installed_modules`
      /// already takes for a malformed manifest). `faults` is handed through unchanged from the
      /// `ValidatorRegistryCache` that calls this, so a rescan never resets an in-flight streak.
      fn scan(modules_dir: &Path, faults: Arc<DashMap<(Uuid, String), u32>>) -> Self {
          let mut by_module = BTreeMap::new();
          let mut config = Config::default();
          config.consume_fuel(true);
          let engine = Engine::new(&config);
          for installed in crate::modules::scan_installed_modules(modules_dir) {
              if installed.validators.is_empty() {
                  continue;
              }
              let module_dir = modules_dir.join(&installed.id);
              let mut result = ModuleLoadResult::default();
              for decl in &installed.validators {
                  match compile_one(&engine, &module_dir, &installed.id, decl) {
                      Ok(compiled) => {
                          result.by_doc_type.insert(decl.doc_type.clone(), compiled);
                      }
                      Err(e) => {
                          tracing::warn!(module = %installed.id, doc_type = %decl.doc_type, error = %e, "validator failed to load");
                          if result.load_error.is_none() {
                              result.load_error = Some(e);
                          }
                      }
                  }
              }
              by_module.insert(installed.id.clone(), result);
          }
          Self { by_module, faults }
      }
  }

  #[cfg(test)]
  impl ValidatorRegistry {
      /// Test-only constructor: a registry whose `validator_for` resolves exactly the given
      /// `(module_id, doc_type)` pairs to the given compiled validators, sharing a fresh
      /// fault-counter map — the same shape `get_or_scan` builds from a real scan, without
      /// touching the filesystem.
      pub(crate) fn for_test(entries: Vec<(&str, &str, CompiledValidator)>) -> Self {
          Self::for_test_with_faults(entries, Arc::default())
      }

      /// As `for_test`, but sharing the given fault-counter map.
      pub(crate) fn for_test_with_faults(
          entries: Vec<(&str, &str, CompiledValidator)>,
          faults: Arc<DashMap<(Uuid, String), u32>>,
      ) -> Self {
          let mut by_module: BTreeMap<String, ModuleLoadResult> = BTreeMap::new();
          for (module_id, doc_type, validator) in entries {
              by_module
                  .entry(module_id.to_string())
                  .or_default()
                  .by_doc_type
                  .insert(doc_type.to_string(), validator);
          }
          Self { by_module, faults }
      }
  }

  /// Reads, traversal-checks and compiles one `ValidatorDecl` under `module_id` — the INSTALLED
  /// module's own id (`installed.id` in `ValidatorRegistry::scan`'s loop), never `decl.doc_type`:
  /// `doc_type` names only which document type this validator judges, and two different modules
  /// may both declare a validator for the SAME `doc_type`, so `doc_type` alone cannot identify
  /// which module a fault belongs to. `MAX_WASM_BYTES` (4 MiB) bounds compile time; the traversal
  /// check is the SAME `is_strictly_within` boundary `http::module_routes::serve_module_file`
  /// enforces at request time, applied here at scan time since this reads the file directly
  /// rather than serving it.
  fn compile_one(
      engine: &Engine,
      module_dir: &Path,
      module_id: &str,
      decl: &crate::modules::ValidatorDecl,
  ) -> Result<CompiledValidator, String> {
      const MAX_WASM_BYTES: u64 = 4 * 1024 * 1024;
      let module_dir_canon = std::fs::canonicalize(module_dir).map_err(|e| e.to_string())?;
      let candidate = module_dir.join(&decl.wasm);
      let candidate_canon = std::fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
      if !crate::http::module_routes::is_strictly_within(&candidate_canon, &module_dir_canon) {
          return Err("wasm path escapes the module's own folder".to_string());
      }
      let meta = std::fs::metadata(&candidate_canon).map_err(|e| e.to_string())?;
      if meta.len() > MAX_WASM_BYTES {
          return Err(format!("validator wasm exceeds {MAX_WASM_BYTES} bytes"));
      }
      let bytes = std::fs::read(&candidate_canon).map_err(|e| e.to_string())?;
      let module = Module::new(engine, &bytes).map_err(|e| e.to_string())?;
      Ok(CompiledValidator {
          module,
          module_id: module_id.to_string(),
      })
  }

  /// Caches `ValidatorRegistry::scan`'s result the same way `crate::modules::ModuleScanCache`
  /// caches a manifest scan: invalidated by the modules directory's own mtime plus each cached
  /// module's `module.json` mtime. One instance per `SqliteRepository` (`SqliteRepository::
  /// validator_registry`), NOT shared with `ModuleScanCache` — the two caches invalidate on the
  /// same signal but hold structurally different payloads (raw manifests vs compiled `wasmi`
  /// modules) and are read from different layers (`ws`/`http` vs `data`).
  #[derive(Default)]
  pub struct ValidatorRegistryCache {
      entry: Mutex<Option<Arc<CachedEntry>>>,
      /// Per-(world, module) consecutive-fault counter, OUTLIVING any single scan — a rescan
      /// (a module installed/removed/changed) rebuilds `entry`'s compiled-module map but must
      /// never reset an in-flight fault streak, so this lives on the cache itself and is handed,
      /// by reference, to every `ValidatorRegistry` `get_or_scan` produces.
      faults: Arc<DashMap<(Uuid, String), u32>>,
  }

  /// One cached scan plus the mtimes it was valid against.
  struct CachedEntry {
      dir_mtime: std::time::SystemTime,
      manifest_mtimes: BTreeMap<String, std::time::SystemTime>,
      registry: Arc<ValidatorRegistry>,
  }

  impl ValidatorRegistryCache {
      /// Clears `(world, module)`'s consecutive-fault counter — a cheap, synchronous `DashMap`
      /// removal, safe to call directly from an async context without `spawn_blocking`. Called
      /// by `SqliteRepository::reset_validator_fault_streak` after `Room::disable_faulting_validator`
      /// finishes disabling the module, so a future re-enable starts clean.
      pub fn reset_faults(&self, world: Uuid, module: &str) {
          self.faults.remove(&(world, module.to_string()));
      }

      /// Returns the cached registry if fresh, else recompiles (blocking; call only from
      /// `spawn_blocking`) and replaces the cache.
      pub fn get_or_scan(&self, modules_dir: &Path) -> Arc<ValidatorRegistry> {
          let dir_mtime = std::fs::metadata(modules_dir).and_then(|m| m.modified()).ok();
          let current = self
              .entry
              .lock()
              .expect("validator registry cache mutex poisoned")
              .clone();
          if let (Some(cached), Some(dir_mtime)) = (current.as_ref(), dir_mtime) {
              if cached.dir_mtime == dir_mtime
                  && cached.manifest_mtimes.iter().all(|(id, mtime)| {
                      std::fs::metadata(modules_dir.join(id).join("module.json"))
                          .and_then(|m| m.modified())
                          .ok()
                          == Some(*mtime)
                  })
              {
                  return cached.registry.clone();
              }
          }
          let registry = Arc::new(ValidatorRegistry::scan(modules_dir, self.faults.clone()));
          let manifest_mtimes = crate::modules::scan_installed_modules(modules_dir)
              .into_iter()
              .filter_map(|m| {
                  std::fs::metadata(modules_dir.join(&m.id).join("module.json"))
                      .and_then(|meta| meta.modified())
                      .ok()
                      .map(|mt| (m.id, mt))
              })
              .collect();
          let fresh = Arc::new(CachedEntry {
              dir_mtime: dir_mtime.unwrap_or(std::time::SystemTime::UNIX_EPOCH),
              manifest_mtimes,
              registry: registry.clone(),
          });
          *self
              .entry
              .lock()
              .expect("validator registry cache mutex poisoned") = Some(fresh);
          registry
      }
  }

  #[cfg(test)]
  mod tests;
  ```
- Create: `src/server/src/sandbox/registry/tests.rs`:
  ```rust
  use super::*;

  #[test]
  fn scan_of_a_missing_modules_dir_yields_an_empty_registry() {
      let registry = ValidatorRegistry::scan(
          std::path::Path::new("no-such-modules-dir"),
          Arc::default(),
      );
      assert!(registry.validator_for("anything", "actor").is_none());
  }

  #[test]
  fn cache_returns_the_same_registry_on_a_second_unchanged_scan() {
      let dir = std::env::temp_dir().join(format!("shadowcat-sandbox-registry-{}", uuid::Uuid::new_v4()));
      std::fs::create_dir_all(&dir).unwrap();
      let cache = ValidatorRegistryCache::default();
      let first = cache.get_or_scan(&dir);
      let second = cache.get_or_scan(&dir);
      assert!(Arc::ptr_eq(&first, &second));
      std::fs::remove_dir_all(&dir).ok();
  }

  /// Writes a minimal installed-module folder under `dir/<id>/` declaring one validator for
  /// `doc_type`, compiled from `wat_src`.
  fn write_installed_module(dir: &std::path::Path, id: &str, doc_type: &str, wat_src: &str) {
      let module_dir = dir.join(id);
      std::fs::create_dir_all(&module_dir).unwrap();
      std::fs::write(
          module_dir.join("module.json"),
          serde_json::json!({
              "id": id,
              "version": "1.0.0",
              "validators": [{ "docType": doc_type, "wasm": "v.wasm" }],
          })
          .to_string(),
      )
      .unwrap();
      let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
      std::fs::write(module_dir.join("v.wasm"), bytes).unwrap();
  }

  /// No `validate` export at all — every call faults with `FaultKind::BadAbi`.
  const FAULTING_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0)))
  "#;

  /// Always accepts.
  const ACCEPTING_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
  "#;

  fn doc(doc_type: &str, system: serde_json::Value) -> crate::data::document::Document {
      crate::data::document::Document {
          id: uuid::Uuid::new_v4(),
          scope: crate::data::document::Scope::World {
              world_id: uuid::Uuid::nil(),
          },
          doc_type: doc_type.into(),
          schema_version: 1,
          name: None,
          source: None,
          base: None,
          owner: None,
          permissions: crate::data::document::PermissionSet::default(),
          embedded: std::collections::BTreeMap::new(),
          parent_id: None,
          engine: None,
          system,
          created_at: 0,
          updated_at: 0,
      }
  }

  #[test]
  fn scan_assigns_each_compiled_validator_the_installed_modules_own_id() {
      // Both modules declare a validator for the SAME doc_type — the only way to prove
      // `compile_one` stamps `module_id` from the INSTALLED module, never from `decl.doc_type`.
      let dir = std::env::temp_dir().join(format!(
          "shadowcat-sandbox-scan-ids-{}",
          uuid::Uuid::new_v4()
      ));
      std::fs::create_dir_all(&dir).unwrap();
      write_installed_module(&dir, "module-a", "actor", FAULTING_WAT);
      write_installed_module(&dir, "module-b", "actor", ACCEPTING_WAT);

      let registry = ValidatorRegistry::scan(&dir, Arc::default());
      let a = registry
          .validator_for("module-a", "actor")
          .expect("module-a's validator compiled");
      let b = registry
          .validator_for("module-b", "actor")
          .expect("module-b's validator compiled");
      assert_eq!(
          a.module_id, "module-a",
          "module_id must be the INSTALLED MODULE id, never the doc_type it validates"
      );
      assert_eq!(b.module_id, "module-b");

      std::fs::remove_dir_all(&dir).ok();
  }

  #[tokio::test]
  async fn validate_document_over_a_scanned_registry_isolates_each_modules_fault_streak() {
      let dir = std::env::temp_dir().join(format!(
          "shadowcat-sandbox-scan-streak-{}",
          uuid::Uuid::new_v4()
      ));
      std::fs::create_dir_all(&dir).unwrap();
      write_installed_module(&dir, "module-a", "actor", FAULTING_WAT);
      write_installed_module(&dir, "module-b", "actor", ACCEPTING_WAT);

      let registry = ValidatorRegistry::scan(&dir, Arc::default());
      let world = uuid::Uuid::from_u128(42);
      let a_only = vec!["module-a".to_string()];
      let b_only = vec!["module-b".to_string()];
      let mut d = doc("actor", serde_json::json!({}));

      for expected in 1..=3u32 {
          let verdict = crate::sandbox::validate_document(&registry, &a_only, &mut d, None, world, &[])
              .await
              .expect("structurally valid document");
          let crate::sandbox::ValidatorVerdict::Fault(fault) = verdict else {
              panic!("expected Fault from module-a, got {verdict:?}");
          };
          assert_eq!(fault.module, "module-a");
          assert_eq!(fault.consecutive, expected);
      }

      // module-b's own accept, resolved through the SAME scanned registry but keyed on a
      // DIFFERENT installed-module id (both modules here declare the SAME doc_type, "actor"),
      // must never touch module-a's streak.
      let verdict = crate::sandbox::validate_document(&registry, &b_only, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      assert_eq!(verdict, crate::sandbox::ValidatorVerdict::Accept);

      let verdict = crate::sandbox::validate_document(&registry, &a_only, &mut d, None, world, &[])
          .await
          .expect("structurally valid document");
      let crate::sandbox::ValidatorVerdict::Fault(fault) = verdict else {
          panic!("expected Fault from module-a, got {verdict:?}");
      };
      assert_eq!(
          fault.consecutive, 4,
          "module-b's accept must not have reset or incremented module-a's streak"
      );

      std::fs::remove_dir_all(&dir).ok();
  }
  ```
  (`std::fs::remove_dir_all` here is a test-only tempdir cleanup of a directory THIS test itself
  created inside the OS temp root, not a project-tracked path — the `trash`-only rule governs
  deletions inside the repo working tree; every other test in this codebase that creates a
  `std::env::temp_dir()` fixture already cleans up the same way, e.g. `Harness::spawn_with`'s
  `assets_dir` is never explicitly deleted either, so this mirrors the existing convention rather
  than introducing a new one.)

- Modify: `src/server/src/modules.rs`:
  - Add, after `struct ModuleEngines` / `fn default_entry`:
    ```rust
    /// One declared validator: which `doc_type` it judges and where its compiled `.wasm` lives,
    /// relative to the module's own install folder.
    #[derive(Debug, Clone, Deserialize)]
    pub struct ValidatorDecl {
        /// The `doc_type` this validator's `system` band judges.
        #[serde(rename = "docType")]
        pub doc_type: String,
        /// Path to the compiled `.wasm`, relative to the module's own folder; a path escaping
        /// that folder (via `..` or an absolute path) is refused at compile time by
        /// `sandbox::registry::compile_one`'s traversal check, the same guard
        /// `http::module_routes::serve_module_file` uses.
        pub wasm: std::path::PathBuf,
    }
    ```
  - Add a `validators` field to `ModuleManifestMirror`, `#[serde(default)]`:
    ```rust
        /// Declared server-side validators, compiled once by `sandbox::registry::ValidatorRegistry`.
        #[serde(default)]
        validators: Vec<ValidatorDecl>,
    ```
  - Add a `validators` field to `InstalledModule`, doc'd, populated from the mirror:
    ```rust
        /// This module's declared validators. Compiled separately by
        /// `sandbox::registry::ValidatorRegistry`; a compile failure is recorded there
        /// (`load_error_for`), never here — discovery always succeeds for a structurally valid
        /// manifest regardless of whether its validators compile.
        pub validators: Vec<ValidatorDecl>,
    ```
    and in `scan_installed_modules`'s `out.push(InstalledModule { .. })` literal, add
    `validators: mirror.validators,`.
  - Update EVERY existing `InstalledModule { .. }` doctest literal in this file (the two shown
    in the earlier reads, at `InstalledModule`'s own doc comment and `engine_compat_ok`'s doc
    comment) to add `validators: vec![],`.
  - Update `src/server/src/modules/tests.rs` (the sibling test file) for any existing
    `InstalledModule { .. }` literal the same way (`rg "InstalledModule \{" src/server/src/modules`).

- Modify: `src/client/core/src/manifest.ts`:
  - Add to `ModuleManifest`, after `systemDefaults?`:
    ```ts
      /** Declared server-side validators (advisory display only — the client never runs one; the
       * server's `modules::scan_installed_modules` reads this authoritatively). */
      validators?: { docType: string; wasm: string }[];
    ```
  - Add to `ManifestSchema`'s `z.object({...})`, after `systemDefaults`:
    ```ts
      validators: z
        .array(z.object({ docType: z.string().min(1), wasm: z.string().min(1) }))
        .optional(),
    ```

- [ ] **Step 1:** write/extend `src/server/src/modules/tests.rs` FIRST with a failing case:
  a manifest declaring `"validators": [{"docType":"actor","wasm":"v.wasm"}]` parses into
  `InstalledModule.validators`; then implement the `modules.rs` changes. Write
  `src/server/src/sandbox/registry/tests.rs` (above) and implement `registry.rs`.
- [ ] **Step 2:** `cargo test --all` (background + log) PASS; both clippy invocations + fmt PASS;
  `pnpm --filter @shadowcat/core test`, `pnpm --filter @shadowcat/core typecheck` PASS.
- [ ] **Step 3:** `git commit -m "feat(sandbox): validator manifest key, InstalledModule.validators, compiled registry" -- src/server/src/modules.rs src/server/src/modules/ src/server/src/sandbox/ src/server/src/http/module_routes.rs src/client/core/src/manifest.ts`

---

### Task 5: `WorldModuleEntry` — per-world enablement record moves from `Vec<String>`

**Files:**
- Create the ts-rs-exported type in `src/server/src/modules.rs`, after `ValidatorDecl`:
  ```rust
  use ts_rs::TS;

  /// A world's enablement record for one installed module: whether it is enabled at all, and
  /// (only meaningful when enabled AND the module declares validators) whether the GM has
  /// additionally opted this world into running its sandboxed validators.
  ///
  /// # Examples
  ///
  /// ```
  /// use shadowcat::modules::WorldModuleEntry;
  ///
  /// let e = WorldModuleEntry { id: "example-module".into(), validators_enabled: false };
  /// assert!(!e.validators_enabled);
  /// ```
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
  #[ts(export, export_to = "../../types/generated/")]
  pub struct WorldModuleEntry {
      /// The installed module's folder id (the same key `InstalledModule::id` uses).
      pub id: String,
      /// Whether this world has opted into running this module's declared validators. `false`
      /// for every entry a legacy bare-string-array setting parses into (back-compat: an
      /// existing dev/production DB's enabled set never silently starts running validators).
      pub validators_enabled: bool,
  }

  impl WorldModuleEntry {
      /// Parses a `world-modules` settings value tolerant of the legacy shape: the new
      /// `Vec<WorldModuleEntry>` array-of-objects first, falling back to a bare
      /// `Vec<String>` (every entry reads as `validators_enabled: false`) when the new shape
      /// fails to parse. Shared by `SqliteRepository::world_enabled_modules` (the live getter)
      /// and `SqliteRepository::import_world` (reading a bundle's own settings row) so the two
      /// call sites can never diverge on what "legacy" means.
      pub fn parse_legacy_tolerant(json: &str) -> Result<Vec<Self>, serde_json::Error> {
          if let Ok(entries) = serde_json::from_str::<Vec<Self>>(json) {
              return Ok(entries);
          }
          let legacy: Vec<String> = serde_json::from_str(json)?;
          Ok(legacy
              .into_iter()
              .map(|id| WorldModuleEntry {
                  id,
                  validators_enabled: false,
              })
              .collect())
      }
  }
  ```
  (`Serialize`/`Deserialize` need importing at the top of `modules.rs` — already imported via
  `use serde::Deserialize;`; add `Serialize` to that `use` line.)

- Modify: `src/server/src/data/repository.rs` — change the trait's `world_enabled_modules`
  return type and add a new `set_world_enabled_modules` trait method, right after it:
  ```rust
      async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<crate::modules::WorldModuleEntry>, DataError>;

      /// Replace a world's enabled installed-module set (GM/admin-authorized by the caller — this
      /// trait method itself performs no authorization). Stored as JSON in `settings`, beside
      /// `world_cap_requirements`/`world_contract_declarations` — enable/disable never mutates
      /// either of those.
      ///
      /// # Examples
      ///
      /// ```
      /// # #[tokio::main]
      /// # async fn main() -> Result<(), shadowcat::data::DataError> {
      /// use shadowcat::data::repository::Repository;
      /// use shadowcat::data::sqlite::SqliteRepository;
      /// use shadowcat::modules::WorldModuleEntry;
      /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
      /// let world = repo.create_world("MOCK_WORLD", 0).await?;
      /// let entries = vec![WorldModuleEntry { id: "mock-module".into(), validators_enabled: false }];
      /// repo.set_world_enabled_modules(world.id, &entries).await?;
      /// assert_eq!(repo.world_enabled_modules(world.id).await?, entries);
      /// # Ok(())
      /// # }
      /// ```
      async fn set_world_enabled_modules(
          &self,
          world: Uuid,
          entries: &[crate::modules::WorldModuleEntry],
      ) -> Result<(), DataError>;
  ```
  Also update the DOC EXAMPLE on the EXISTING `world_enabled_modules` trait method (the one
  reading `let ids = repo.world_enabled_modules(...)`) to use `WorldModuleEntry` instead of
  a bare string vec, and `assert!(ids.is_empty())` stays valid unchanged.

- Modify: `src/server/src/data/sqlite.rs` — change the `impl Repository for SqliteRepository`
  block's `world_enabled_modules` (found via `rg "async fn world_enabled_modules"
  src/server/src/data/sqlite.rs`):
  ```rust
      async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<crate::modules::WorldModuleEntry>, DataError> {
          match self.get_setting(&world_modules_key(world)).await? {
              Some(json) => Ok(crate::modules::WorldModuleEntry::parse_legacy_tolerant(&json)?),
              None => Ok(Vec::new()),
          }
      }

      async fn set_world_enabled_modules(
          &self,
          world: Uuid,
          entries: &[crate::modules::WorldModuleEntry],
      ) -> Result<(), DataError> {
          let json = serde_json::to_string(entries)?;
          self.set_setting(&world_modules_key(world), &json).await
      }
  ```
- Modify: `src/server/src/data/sqlite/worlds.rs` — DELETE the inherent
  `pub async fn set_world_enabled_modules` method entirely (its doc-comment example moves,
  updated to the new type, into the trait method's doc comment above — do not leave a
  duplicate).

- Modify: `src/server/src/data/world_seed.rs`'s `enabled_system_defaults`:
  ```rust
      let enabled = match repo.world_enabled_modules(world_id).await {
          Ok(e) => e,
          Err(e) => {
              tracing::warn!(world = %world_id, error = %e, "enabled-module read failed; config seed proceeds without a system layer");
              return None;
          }
      };
      if enabled.is_empty() {
          return None;
      }
      scan_installed_modules(modules_dir)
          .into_iter()
          .find(|m| m.provides_system && enabled.iter().any(|e| e.id == m.id))
          .and_then(|m| m.system_defaults)
  ```
  (only the `enabled.iter().any(|id| id == &m.id)` line changes to `enabled.iter().any(|e| e.id == m.id)`).
  Update `data/world_seed/tests.rs:189`'s `r.set_world_enabled_modules(w.id, &["sys".to_string()])`
  to `r.set_world_enabled_modules(w.id, &[crate::modules::WorldModuleEntry { id: "sys".into(),
  validators_enabled: false }])`.

- Modify: `src/server/src/ws/conn.rs`'s `welcome_capability_requirements` — the
  `for id in &enabled { ... if let Some(m) = installed.iter().find(|m| &m.id == id && ...` loop
  becomes `for entry in &enabled { ... if let Some(m) = installed.iter().find(|m| m.id == entry.id
  && crate::modules::engine_compat_ok(m)) { ... } }` (rename the loop variable; body unchanged
  otherwise).
- Modify: `src/server/src/ws/room/tests/mod.rs` — the `DeleteMidHydration` mock's
  `world_enabled_modules` becomes:
  ```rust
      async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<crate::modules::WorldModuleEntry>, DataError> {
          self.inner.world_enabled_modules(world).await
      }

      async fn set_world_enabled_modules(
          &self,
          world: Uuid,
          entries: &[crate::modules::WorldModuleEntry],
      ) -> Result<(), DataError> {
          self.inner.set_world_enabled_modules(world, entries).await
      }
  ```

- Modify: `src/server/src/http/module_routes.rs`:
  - `get_world_enabled_modules` return type: `Result<Json<Vec<crate::modules::WorldModuleEntry>>,
    AppError>`; body unchanged (`Ok(Json(state.repo.world_enabled_modules(world).await?))`).
    Update its doc example's `let ids = ...; assert!(ids.0.is_empty())` — unchanged, still
    compiles against the new type.
  - `set_world_enabled_modules` handler: change `Json(ids): Json<Vec<String>>` to
    `Json(entries): Json<Vec<crate::modules::WorldModuleEntry>>`; replace every subsequent `ids`
    reference:
    ```rust
    pub async fn set_world_enabled_modules(
        user: AuthUser,
        State(state): State<AppState>,
        Path(world): Path<Uuid>,
        Json(entries): Json<Vec<crate::modules::WorldModuleEntry>>,
    ) -> Result<StatusCode, AppError> {
        require_gm(&state, &user, world).await?;
        if entries.len() > MAX_ENABLED_MODULES {
            return Err(AppError::Unprocessable(format!(
                "too many enabled modules (max {MAX_ENABLED_MODULES})"
            )));
        }
        let mut seen = std::collections::HashSet::with_capacity(entries.len());
        let entries: Vec<crate::modules::WorldModuleEntry> = entries
            .into_iter()
            .filter(|e| seen.insert(e.id.clone()))
            .collect();
        let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
        for entry in &entries {
            let Some(m) = installed.iter().find(|m| m.id == entry.id) else {
                return Err(AppError::Unprocessable(format!(
                    "module '{}' is not installed",
                    entry.id
                )));
            };
            if !crate::modules::engine_compat_ok(m) {
                return Err(AppError::Unprocessable(format!(
                    "module '{}' is incompatible with this server version (requires shadowcat {})",
                    entry.id,
                    m.engines_shadowcat.as_deref().unwrap_or("(missing engines.shadowcat)")
                )));
            }
            // An enabled module with no declared validators can never meaningfully opt in.
            if entry.validators_enabled && m.validators.is_empty() {
                return Err(AppError::Unprocessable(format!(
                    "module '{}' declares no validators",
                    entry.id
                )));
            }
        }
        let systems: Vec<&str> = entries
            .iter()
            .filter(|e| installed.iter().any(|m| m.id == e.id && m.provides_system))
            .map(|e| e.id.as_str())
            .collect();
        if systems.len() > 1 {
            return Err(AppError::Unprocessable(format!(
                "at most one enabled module may provide {} (got: {})",
                crate::modules::SYSTEM_CONTRACT,
                systems.join(", ")
            )));
        }
        state.repo.set_world_enabled_modules(world, &entries).await?;
        match state.ws.rooms.get_or_create(state.repo.as_ref(), world).await {
            Ok(Some(room)) => {
                if let Err(e) = crate::ws::conn::reseed_world_config(
                    &room,
                    state.repo.as_ref(),
                    &state.config.modules_path(),
                )
                .await
                {
                    tracing::warn!(world = %world, error = %e, "system-defaults refresh failed after enable-set change");
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(world = %world, error = %e, "room open for system-defaults refresh failed");
            }
        }
        Ok(StatusCode::NO_CONTENT)
    }
    ```
  - Update this function's doc example's `Json(vec!["dnd5e".to_string()])` to
    `Json(vec![crate::modules::WorldModuleEntry { id: "dnd5e".into(), validators_enabled: false }])`.
  - Add `has_validators: bool` and `validator_load_error: Option<String>` to
    `InstalledModuleInfo`, and thread them through `impl From<&InstalledModule> for
    InstalledModuleInfo` — this `From` impl has no access to the registry, so it takes the
    registry as a second argument; change every call site accordingly:
    ```rust
    #[derive(Debug, Clone, Serialize, TS)]
    #[ts(export, export_to = "../../types/generated/")]
    pub struct InstalledModuleInfo {
        pub id: String,
        #[ts(type = "unknown")]
        pub manifest: serde_json::Value,
        pub entry_url: String,
        /// Whether this module declares any validators (`InstalledModule.validators` non-empty).
        pub has_validators: bool,
        /// This module's validator compile diagnostic, if any of its declared validators failed
        /// to load (`sandbox::registry::ValidatorRegistry::load_error_for`) — shown beside the
        /// "Run sandboxed validators" toggle.
        pub validator_load_error: Option<String>,
    }

    impl InstalledModuleInfo {
        /// Projects `m` for the wire, consulting `registry` for its validator load status.
        fn from_installed(
            m: &crate::modules::InstalledModule,
            registry: &crate::sandbox::registry::ValidatorRegistry,
        ) -> Self {
            InstalledModuleInfo {
                id: m.id.clone(),
                manifest: m.manifest_json.clone(),
                entry_url: m.entry_url.clone(),
                has_validators: !m.validators.is_empty(),
                validator_load_error: registry.load_error_for(&m.id).map(str::to_string),
            }
        }
    }
    ```
    (delete the old `impl From<&crate::modules::InstalledModule> for InstalledModuleInfo`
    entirely — it is superseded by `from_installed`, which needs the registry the bare `From`
    trait cannot carry).
  - `list_installed_modules` becomes:
    ```rust
    pub async fn list_installed_modules(
        _user: AuthUser,
        State(state): State<AppState>,
    ) -> Json<Vec<InstalledModuleInfo>> {
        let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
        let registry = state.repo.validator_registry(&state.config.modules_path());
        Json(
            installed
                .iter()
                .map(|m| InstalledModuleInfo::from_installed(m, &registry))
                .collect(),
        )
    }
    ```
    (`state.repo.validator_registry(&Path)` is added to `SqliteRepository` in Task 6 as a thin
    wrapper over `self.validator_registry_cache.get_or_scan(modules_dir)`; this task adds the
    call site here and Task 6 adds the method — if Task 6 has not landed yet when this task's
    gates run, stub it inline as `Arc::new(crate::sandbox::registry::ValidatorRegistry::default())`
    and remove the stub in Task 6's diff. Given these two tasks land in the SAME PR sequence
    before any push, prefer implementing Task 6's `validator_registry` method now, in this task,
    to avoid the stub-and-revert churn — see Task 6's `SqliteRepository` fields, which this task
    may add early if convenient.)
  - Update this route's doc example's `InstalledModuleInfo { id: ..., manifest: ...,
    entry_url: ... }` literal to add `has_validators: false, validator_load_error: None`.

- Modify: `src/client/core/src/module-rest.ts`:
  ```ts
  import type { InstalledModuleInfo, WorldModuleEntry } from "@shadowcat/types";

  // ... listInstalledModules unchanged ...

  /** A world's enabled installed-module entries (id + validators_enabled). Any world member may
   * read this (needed at join to load the enabled set).
   * @param world The world's id.
   * @returns The world's currently-enabled module entries.
   * @example
   * ```ts
   * import { getEnabledModules } from "@shadowcat/core";
   *
   * const entries = await getEnabledModules("00000000-0000-0000-0000-000000000001");
   * ```
   */
  export async function getEnabledModules(world: string): Promise<WorldModuleEntry[]> {
    const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
      headers: { accept: "application/json" },
    });
    if (!res.ok) throw new Error(`get enabled modules failed: ${res.status}`);
    return (await res.json()) as WorldModuleEntry[];
  }

  /** Replace a world's enabled installed-module set. GM/admin only server-side.
   * @param world The world's id.
   * @param entries The new enabled-module set (id + validators_enabled per entry).
   * @example
   * ```ts
   * import { setEnabledModules } from "@shadowcat/core";
   *
   * await setEnabledModules("00000000-0000-0000-0000-000000000001", [
   *   { id: "example-module", validators_enabled: false },
   * ]);
   * ```
   */
  export async function setEnabledModules(world: string, entries: WorldModuleEntry[]): Promise<void> {
    const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(entries),
    });
    if (!res.ok) throw new Error(`set enabled modules failed: ${res.status}`);
  }
  ```
- Modify: `src/types/index.ts` — add
  `export type { WorldModuleEntry } from "./generated/WorldModuleEntry";` (regenerated by
  `cargo test --all`; stage it after regen).

- Modify: `src/modules/settings/src/ModuleManager.svelte` — replace the `enabled: Set<string>`
  model with a `Map<string, boolean>` (id → validators_enabled) and add the second toggle:
  ```svelte
  <script lang="ts">
    import { getAppContext } from "@shadowcat/ui-kit";
    import {
      listInstalledModules,
      getEnabledModules,
      setEnabledModules,
      type InstalledModuleInfo,
    } from "@shadowcat/core";

    const { world, t, reconcileInstalledModules } = getAppContext();

    let installed = $state<InstalledModuleInfo[]>([]);
    let enabled = $state<Map<string, boolean>>(new Map());
    let loaded = $state(false);
    let saving = $state(false);
    let error = $state<string | null>(null);

    function displayName(info: InstalledModuleInfo): string {
      const id = (info.manifest as { id?: unknown }).id;
      return typeof id === "string" ? id : info.id;
    }

    async function load(): Promise<void> {
      error = null;
      try {
        const [inst, en] = await Promise.all([listInstalledModules(), getEnabledModules(world)]);
        installed = inst;
        enabled = new Map(en.map((e) => [e.id, e.validators_enabled]));
      } catch (e) {
        error = e instanceof Error ? e.message : String(e);
      } finally {
        loaded = true;
      }
    }
    load();

    function toggle(id: string): void {
      const next = new Map(enabled);
      if (next.has(id)) next.delete(id);
      else next.set(id, false);
      enabled = next;
    }

    /**
     * Flips `id`'s locally-held `validators_enabled` bit (not yet persisted). A module not
     * currently enabled has no entry to flip — the checkbox rendering below only shows this
     * control for an enabled, validator-declaring module, so this is unreachable otherwise.
     * @param id The module's install-folder id.
     * @example
     * ```
     * // private function; not part of the public API — wired to the validators checkbox
     * toggleValidators("example-module");
     * ```
     */
    function toggleValidators(id: string): void {
      const next = new Map(enabled);
      const current = next.get(id);
      if (current === undefined) return;
      next.set(id, !current);
      enabled = next;
    }

    async function save(): Promise<void> {
      saving = true;
      error = null;
      try {
        const entries = [...enabled].map(([id, validators_enabled]) => ({ id, validators_enabled }));
        await setEnabledModules(world, entries);
        await reconcileInstalledModules();
      } catch (e) {
        error = e instanceof Error ? e.message : String(e);
      } finally {
        saving = false;
      }
    }
  </script>

  <section class="module-manager">
    <h3>{t("settings.modules.title")}</h3>
    {#if !loaded}
      <p>{t("settings.modules.loading")}</p>
    {:else if installed.length === 0}
      <p>{t("settings.modules.empty")}</p>
    {:else}
      <ul>
        {#each installed as info (info.id)}
          <li>
            <label>
              <input
                type="checkbox"
                aria-label={displayName(info)}
                checked={enabled.has(info.id)}
                onchange={() => toggle(info.id)}
              />
              {displayName(info)}
            </label>
            {#if info.has_validators && enabled.has(info.id)}
              <label>
                <input
                  type="checkbox"
                  aria-label={t("settings.modules.runValidators", { name: displayName(info) })}
                  checked={enabled.get(info.id) ?? false}
                  onchange={() => toggleValidators(info.id)}
                />
                {t("settings.modules.runValidators", { name: displayName(info) })}
              </label>
              {#if info.validator_load_error}
                <p class="error">{t("settings.modules.validatorLoadError", { message: info.validator_load_error })}</p>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
      <button onclick={save} disabled={saving}>{t("settings.modules.save")}</button>
    {/if}
    {#if error}
      <p class="error">{t("settings.modules.error", { message: error })}</p>
    {/if}
  </section>

  <style lang="scss">
    .module-manager {
      display: grid;
      gap: var(--space-2);
    }
    .error {
      color: var(--danger);
    }
    input[type="checkbox"] {
      min-width: 36px;
      min-height: 36px;
    }
    button {
      min-height: 32px;
    }
  </style>
  ```
- Modify: `src/client/ui-kit/src/locales/en.ts` — add, after `"settings.modules.error"`:
  ```ts
    "settings.modules.runValidators": "Run sandboxed validators ({name})",
    "settings.modules.validatorLoadError": "Validator failed to load: {message}",
  ```
- Modify: `src/modules/settings/src/ModuleManager.test.ts` — every `InstalledModuleInfo` mock
  literal gains `has_validators: false, validator_load_error: null` (add it to the
  `listInstalledModules` mock at line 11 and the two `mockResolvedValueOnce` literals at lines
  94-96); every `getEnabledModules` mock resolves `WorldModuleEntry[]` (an empty array is
  already valid under the new type, no change needed for `mockResolvedValue([])`); every
  `setEnabledModules` assertion changes from a bare string array to entry objects, e.g. line 30's
  `toHaveBeenCalledWith("w1", ["example-system"])` becomes `toHaveBeenCalledWith("w1", [{ id:
  "example-system", validators_enabled: false }])`, and line 105 similarly with `"folder-name"`.
  Add two NEW test cases:
  ```ts
  it("shows the validators toggle only for an enabled module that declares validators", async () => {
    const { listInstalledModules, getEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js", has_validators: true, validator_load_error: null },
    ]);
    vi.mocked(getEnabledModules).mockResolvedValueOnce([{ id: "example-system", validators_enabled: false }]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    expect(await screen.findByLabelText("settings.modules.runValidators")).toBeTruthy();
  });

  it("saves validators_enabled when the sandbox toggle is flipped", async () => {
    const { listInstalledModules, getEnabledModules, setEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js", has_validators: true, validator_load_error: null },
    ]);
    vi.mocked(getEnabledModules).mockResolvedValueOnce([{ id: "example-system", validators_enabled: false }]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const validatorsToggle = await screen.findByLabelText("settings.modules.runValidators");
    await fireEvent.click(validatorsToggle);
    await fireEvent.click(screen.getByText("settings.modules.save"));

    await vi.waitFor(() =>
      expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", [
        { id: "example-system", validators_enabled: true },
      ]),
    );
  });
  ```
  (`findByLabelText("settings.modules.runValidators")` matches because the test's `t()` stub —
  confirm via `setAppContextForTest`'s fixture — returns the raw key when no interpolation
  fixture is registered, the same pattern every other `t("...")` assertion in this file already
  relies on.)

- [ ] **Step 1:** write the failing Rust tests (Repository trait round-trip, legacy-string
  fallback) in `src/server/src/data/sqlite/tests/search_and_worlds.rs`'s existing
  `world_enabled_modules_round_trip` test (update it to the new type) plus a NEW test
  `world_enabled_modules_reads_a_legacy_string_array_as_validators_disabled` asserting
  `set_setting` writing a raw `["mock-module"]` JSON string is read back as
  `[WorldModuleEntry { id: "mock-module".into(), validators_enabled: false }]`. Write the failing
  TS tests. Then implement every file above.
- [ ] **Step 2:** `cargo test --all` (background + log; regenerates `WorldModuleEntry.ts`)
  PASS; `git diff --exit-code src/types/generated` FAILS as expected — stage the new file;
  `pnpm -r typecheck`, `pnpm --filter @shadowcat/core test`, `pnpm --filter
  @shadowcat/module-settings test`, `pnpm lint:docs`, `pnpm lint:props`, both clippy invocations,
  `cargo fmt --check` PASS.
- [ ] **Step 3:** `git commit -m "feat(modules): per-world validators_enabled opt-in (WorldModuleEntry replaces the bare enabled-id list)" -- src/server/src/modules.rs src/server/src/data/ src/server/src/ws/ src/server/src/http/module_routes.rs src/types/ src/client/core/src/module-rest.ts src/client/ui-kit/src/locales/en.ts src/modules/settings/`

---

### Task 6: `apply_intent` wiring — the pre-transaction validator chokepoint

**Files:**
- Modify: `src/server/src/data/DataError` (`src/server/src/data/mod.rs`) — add, after
  `SchemaViolation`:
  ```rust
      /// A sandboxed validator TECHNICALLY failed (trap/out-of-fuel/malformed module) — distinct
      /// from an authored refusal, which is `OpFailed`. `ws::conn`'s `reject_reason` maps this to
      /// `RejectReason::Invalid` like `OpFailed`; the CALLER (never `data` itself — see
      /// `crate::sandbox`'s doc) additionally compares the carried `consecutive` count against
      /// `crate::sandbox::VALIDATOR_FAULT_LIMIT` to decide whether to call
      /// `Room::disable_faulting_validator`.
      #[error("validator '{module}' faulted", module = .0.module)]
      Validator(crate::sandbox::ValidatorFault),
  ```
  (`module = .0.module` is thiserror's documented shorthand for extracting a named field from a
  tuple variant's own field — `.0` expands to `self.0`; verified against the vendored `thiserror`
  version this crate resolves.)
- Modify: `src/server/src/http/error.rs`'s `From<DataError> for AppError` — add, after
  `OpFailed(m) => AppError::BadRequest(m),`:
  ```rust
              Validator(fault) => {
                  AppError::BadRequest(format!("validator '{}' faulted", fault.module))
              }
  ```
- Modify: `src/server/src/data/sqlite.rs`:
  - `SqliteRepository` struct gains two fields, after `connect_options`:
    ```rust
        /// Installed-modules discovery root, `None` when this repository was never wired to one
        /// (every existing test construction via `connect()` alone) — validators never run
        /// without it, fail-open by absence exactly like `scan_installed_modules`'s own missing-
        /// dir handling.
        modules_dir: Option<std::path::PathBuf>,
        /// Compiled validator cache, mirroring `crate::modules::ModuleScanCache`'s own
        /// invalidation. Always present (cheap to construct; does no I/O until first scan).
        validator_registry_cache: std::sync::Arc<crate::sandbox::registry::ValidatorRegistryCache>,
    ```
  - `connect()`'s `Ok(Self { pool, connect_options })` becomes
    `Ok(Self { pool, connect_options, modules_dir: None, validator_registry_cache:
    Default::default() })`.
  - Add, after `open_read_pool`:
    ```rust
        /// Attaches an installed-modules discovery root, enabling sandboxed validator support
        /// on this repository. Every existing `connect()` caller that never calls this keeps
        /// `modules_dir: None` — validators never run, exactly as if none were installed.
        pub fn with_modules_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
            self.modules_dir = Some(dir.into());
            self
        }

        /// The compiled validator registry for this repository's `modules_dir`, or an empty
        /// registry when none is configured. Blocking filesystem I/O on a cache miss — call only
        /// from `spawn_blocking` (mirrors `crate::modules::ModuleScanCache::get_or_scan`).
        fn validator_registry_blocking(&self) -> std::sync::Arc<crate::sandbox::registry::ValidatorRegistry> {
            match &self.modules_dir {
                Some(dir) => self.validator_registry_cache.get_or_scan(dir),
                None => Default::default(),
            }
        }

        /// Public accessor for `http::module_routes::list_installed_modules`'s validator-status
        /// projection — off the async worker via `spawn_blocking`, matching every other blocking
        /// module-scan call site in this crate.
        pub async fn validator_registry(
            &self,
            modules_dir: &std::path::Path,
        ) -> std::sync::Arc<crate::sandbox::registry::ValidatorRegistry> {
            let cache = self.validator_registry_cache.clone();
            let dir = modules_dir.to_path_buf();
            tokio::task::spawn_blocking(move || cache.get_or_scan(&dir))
                .await
                .unwrap_or_default()
        }
    ```
    (`Task 5`'s `list_installed_modules` call site `state.repo.validator_registry(&state.config
    .modules_path())` now resolves against this method — this task supersedes the Task-5 stub
    note; if Task 5 landed the stub literally, replace it here with a call to this method.)
  - Add the shared merge helper, near `apply_field_change`'s other free functions in this file
    (top-level, not inside `impl SqliteRepository`):
    ```rust
    /// Reconstructs the MERGED post-image document a `changes` Update would produce against
    /// `doc_id`'s CURRENT stored row, read through `executor` — shared by Phase 2's authoritative
    /// merge (`&mut *tx`, inside the write transaction) and the pre-transaction validator
    /// pre-image build (a read-only pool connection, before the transaction opens): both merges
    /// must reach the IDENTICAL document, or the validated post-image and the committed one could
    /// silently diverge. Returns the PRE-image and the merged POST-image. `DataError::NotFound` if
    /// the row is absent; `DataError::OpFailed` if `changes` would change the document id.
    async fn merge_update_document<'e, E>(
        executor: E,
        doc_id: Uuid,
        changes: &[FieldChange],
    ) -> Result<(Document, Document), DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(doc_id.to_string())
            .fetch_optional(executor)
            .await?
            .ok_or(DataError::NotFound)?;
        let pre_value: serde_json::Value = serde_json::from_str(row.get::<String, _>("json").as_str())?;
        let pre_doc: Document = serde_json::from_value(pre_value.clone())?;
        let mut post_value = pre_value;
        for ch in changes {
            apply_field_change(&mut post_value, ch)?;
        }
        let post_doc: Document = serde_json::from_value(post_value)?;
        if post_doc.id != doc_id {
            return Err(DataError::OpFailed(
                "update must not change the document id".into(),
            ));
        }
        Ok((pre_doc, post_doc))
    }
    ```
  - REPLACE Phase 2's existing inline load-and-merge block in the `Operation::Update` arm — the
    code from `let row = sqlx::query("SELECT json FROM documents WHERE id = ?")` through
    `let mut doc: Document = serde_json::from_value(value)?;` and its immediately-following
    `if doc.id != *doc_id { return Err(...) }` check — with:
    ```rust
                    let (pre_doc, mut doc) =
                        Self::merge_update_document(&mut *tx, *doc_id, changes).await?;
                    let pre_engine = pre_doc.engine.clone();
    ```
    (`merge_update_document` is a free function, not an inherent method, so this call site reads
    `merge_update_document(&mut *tx, *doc_id, changes).await?` directly — NOT `Self::`; adjust
    accordingly. `pre_engine` replaces the old `value.get("engine").cloned()` — identical value,
    since `pre_doc` deserializes from the exact same pre-merge JSON `value` used to hold.
    Everything AFTER this point in the existing Update arm — `check_command_scope(&doc,
    world_id)?;` onward — is UNCHANGED.)
  - Insert the validator pre-check in `apply_intent`, immediately after the existing block
    (found in the earlier read at lines 807-813):
    ```rust
        let world_schemas = self.world_schema_declarations(world_id).await?;
    ```
    and BEFORE `let mut tx = self.pool.begin().await?;`, add:
    ```rust
        // Sandboxed validators run HERE, entirely before the write transaction opens: the
        // single-writer pool (`max_connections(1)`) serializes every `apply_intent` server-wide,
        // so a validator held inside the transaction would throttle every hosted world. A stale
        // pre-image read here is safe — the OCC pre-image check inside Phase 1 below refuses the
        // write with `Conflict` if the document changed since this read, so the validated
        // post-image is the one that actually commits.
        if let Some(modules_dir) = self.modules_dir.clone() {
            let enabled = self.world_enabled_modules(world_id).await.unwrap_or_default();
            let enabled_module_ids: Vec<String> = enabled
                .iter()
                .filter(|e| e.validators_enabled)
                .map(|e| e.id.clone())
                .collect();
            if !enabled_module_ids.is_empty() {
                let registry = {
                    let cache = self.validator_registry_cache.clone();
                    tokio::task::spawn_blocking(move || cache.get_or_scan(&modules_dir))
                        .await
                        .unwrap_or_default()
                };
                let read_pool = self.open_read_pool().await?;
                for op in &ops {
                    let (mut doc, prior): (Document, Option<Document>) = match op {
                        Operation::Create { doc } => (doc.clone(), None),
                        Operation::Update { doc_id, changes } => {
                            let touches_system = changes
                                .iter()
                                .any(|c| c.path == "/system" || c.path.starts_with("/system/"));
                            if !touches_system {
                                continue;
                            }
                            match Self::merge_update_document(&read_pool, *doc_id, changes).await {
                                Ok((pre, post)) => (post, Some(pre)),
                                // A missing/malformed pre-image here is not this pass's problem to
                                // report — the real, authoritative Phase 1 load inside the
                                // transaction below will surface the SAME failure properly.
                                Err(_) => continue,
                            }
                        }
                        Operation::Move { .. } => continue,
                        Operation::Delete { .. } => continue,
                    };
                    match crate::sandbox::validate_document(
                        &registry,
                        &enabled_module_ids,
                        &mut doc,
                        prior.as_ref(),
                        world_id,
                        &world_schemas,
                    )
                    .await
                    {
                        // `validate_document`'s own structural pre-pass rejected `doc` before any
                        // validator ran — Phase 1 below would reject it identically, so its error
                        // is returned untouched here.
                        Err(structural_err) => return Err(structural_err),
                        Ok(crate::sandbox::ValidatorVerdict::Accept) => {}
                        Ok(crate::sandbox::ValidatorVerdict::Refuse { module, reason }) => {
                            return Err(DataError::OpFailed(format!(
                                "validator {module}: {reason}"
                            )));
                        }
                        Ok(crate::sandbox::ValidatorVerdict::Fault(fault)) => {
                            return Err(DataError::Validator(fault));
                        }
                    }
                }
            }
        }
    ```
    (`Self::merge_update_document(&read_pool, ...)` here passes `&SqlitePool` as the generic
    `E: sqlx::Executor` — `&SqlitePool` implements `Executor` by reference exactly like `&mut
    *tx` does, so the same free function serves both call sites.)

- [ ] **Step 1:** write failing tests FIRST in
  `src/server/src/data/sqlite/tests/commands_and_intents.rs` (the sibling test file
  `apply_intent`'s other tests already live in — confirm via `rg "mod commands_and_intents"
  src/server/src/data/sqlite/tests`), using `SqliteRepository::connect("sqlite::memory:").await
  .unwrap().with_modules_dir(<tempdir with a hand-built module.json + wasm>)`: a Create with a
  validator-refused `system` is rejected with an `OpFailed` message containing `"validator "`;
  `validators_enabled: false` on the world's entry ⇒ accepted regardless of the module's
  verdict; the module itself disabled (absent from the enabled list) ⇒ accepted; an embedded
  child validated under its own `doc_type`; `apply_command` (the trusted replay path) NEVER
  invokes a validator even when the world has one enabled+opted-in (assert by constructing a
  refusing validator and calling `apply_command` directly — it must succeed); two modules
  enabled in REVERSE-alphabetical order (`["module-b", "module-a"]`), both refusing, ⇒ the
  alphabetically-first module's (`module-a`'s) reason is the one returned — proving
  `sandbox::validate_document` sorts the enabled set itself rather than trusting `apply_intent`'s
  own collection order; a Create whose `system` violates a GM-declared tier-2 schema
  (`set_world_schema_declarations`) is rejected with `DataError::SchemaViolation`, never
  `DataError::Validator`, even when the world also has an ALWAYS-FAULTING validator
  enabled+opted-in for that `doc_type` — and calling `SqliteRepository::validator_registry`'s
  returned registry's `record_fault(world, module_id)` immediately afterward reads `1`, proving
  the rejected submission never reached, and never faulted, that validator; a document changed
  between the pre-image read and the transaction ⇒ `Conflict`: capture an Update's
  `FieldChange.old` from the
  document's value at time T0, then commit a DIFFERENT `apply_intent` Update to that same
  `/system` path (accepted by the validator) so the stored value moves to T1, THEN call
  `apply_intent` with the T0-captured `old` — Phase 1's ordinary OCC pre-image check (not
  anything sandbox-specific) rejects it with `Conflict` before the validator's own pre-tx read
  is even consulted for correctness, which is exactly the guarantee this test pins: the
  validator ran against a since-superseded pre-image, and the OCC check caught it anyway before
  that pre-image's validated post-image could commit. Build the WAT-compiled test fixture module
  bytes with the `wat` crate
  directly in the test (no need for the full `examples/validator-rust` crate here — Task 10 owns
  the REAL example; these unit tests use a tiny inline WAT accept/refuse pair matching Task 3's
  fixtures' ABI).
- [ ] **Step 2:** implement every file above. `cargo test --all` (background + log) PASS; both
  clippy invocations + fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(sandbox): wire validate_document into apply_intent, outside the write transaction" -- src/server/src/data/ src/server/src/http/error.rs`

---

### Task 7: `import_world` wiring; production/test-harness `modules_dir` plumbing

**Files:**
- Modify: `src/server/src/data/sqlite/export_import.rs`'s `import_world` — add, right after
  the existing `world_schemas` derivation block (the code reading
  `world_schemas_key(world)` from `data.settings`):
  ```rust
        let enabled_module_ids: Vec<String> = data
            .settings
            .iter()
            .find(|s| s.key == world_modules_key(world))
            .map(|s| crate::modules::WorldModuleEntry::parse_legacy_tolerant(&s.value))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.validators_enabled)
            .map(|e| e.id)
            .collect();
        let validator_registry = if enabled_module_ids.is_empty() {
            None
        } else {
            self.modules_dir.clone().map(|dir| self.validator_registry_cache.get_or_scan(&dir))
        };
  ```
  and inside the `for row in ordered_rows` loop, immediately after
  `validation::validate_system_schema_tree(&document, &world_schemas)?;` and BEFORE
  `Self::insert_imported_document(...)`, add:
  ```rust
            if let Some(registry) = &validator_registry {
                match crate::sandbox::validate_document(
                    registry,
                    &enabled_module_ids,
                    &mut document,
                    None,
                    world,
                    &world_schemas,
                )
                .await
                {
                    // `validate_document`'s structural pre-pass also runs `validate_containment`,
                    // which this loop's own inline chain above does not call until the post-loop
                    // placement pass below — so a containment violation can surface HERE, before
                    // any row is inserted, rather than only after the whole bundle is written.
                    // `validate_containment` needs no other document's state to decide, so an
                    // earlier verdict is identical to the later one, just reached sooner.
                    Err(structural_err) => return Err(structural_err),
                    Ok(crate::sandbox::ValidatorVerdict::Accept) => {}
                    Ok(crate::sandbox::ValidatorVerdict::Refuse { module, reason }) => {
                        return Err(DataError::OpFailed(format!(
                            "validator {module}: {reason}"
                        )));
                    }
                    Ok(crate::sandbox::ValidatorVerdict::Fault(fault)) => {
                        return Err(DataError::Validator(fault));
                    }
                }
            }
  ```
  (`import_world` already holds `tx` exclusively for its entire duration by design — see this
  function's own doc comment, "Holds the pool's single writer connection for the entire call" —
  so running validators inside its loop, unlike `apply_intent`'s per-write hot path, adds no NEW
  throttling: nothing else can write concurrently either way. `validator_registry_cache.get_or_scan`
  is blocking filesystem I/O called here without `spawn_blocking`, matching `import_world`'s own
  existing posture of running entirely synchronously-styled blocking DB work under one held
  transaction — this is an accepted, pre-existing trade-off of the function, not a new one this
  task introduces.)
- Import `world_modules_key` at the top of `export_import.rs` if not already imported (`rg
  "world_modules_key" src/server/src/data/sqlite/export_import.rs` first — it likely needs
  `use super::world_modules_key;` alongside the existing `world_schemas_key` import).

- Modify: `src/server/src/main.rs` line 53:
  ```rust
      let repo = SqliteRepository::connect(&config.db).await?.with_modules_dir(config.modules_path());
  ```
- Modify: `src/server/src/bin/test_server.rs` — move the `Args::parse()` + `Config` construction
  (currently at lines 163-167) to the TOP of `main()`, before `repo` is constructed, and wire it:
  ```rust
  async fn main() -> anyhow::Result<()> {
      tracing_subscriber::fmt().with_env_filter("info").init();
      let args = Args::parse();
      let mut config = Config::default();
      if let Some(dir) = args.modules_dir {
          config.modules_dir = Some(dir);
      }
      let repo = Arc::new(
          SqliteRepository::connect("sqlite::memory:")
              .await?
              .with_modules_dir(config.modules_path()),
      );
      let hash = hash_password("pw")?;
      // ... unchanged body through the existing fixture-seeding code ...
  ```
  DELETE the now-duplicate `let args = Args::parse(); let mut config = Config::default(); if let
  Some(dir) = args.modules_dir { config.modules_dir = Some(dir); }` block that used to sit right
  before `let state = AppState { ... };` — `config` is now the one built at the top; everything
  from `let state = AppState {` onward is otherwise unchanged.
- Modify: `src/server/test-support/src/lib.rs`'s `spawn_with`:
  ```rust
  pub async fn spawn_with(mutate: impl FnOnce(&mut Config)) -> Harness {
      let assets_dir = std::env::temp_dir().join(format!("shadowcat-assets-{}", Uuid::new_v4()));
      std::fs::create_dir_all(&assets_dir).unwrap();
      let mut cfg = Config {
          assets_dir: Some(assets_dir.to_string_lossy().into_owned()),
          ..Config::default()
      };
      mutate(&mut cfg);

      let repo = Arc::new(
          SqliteRepository::connect("sqlite::memory:")
              .await
              .unwrap()
              .with_modules_dir(cfg.modules_path()),
      );
      let hash = hash_password("pw").unwrap();
      let uid = repo
          .create_user("u", Some(&hash), ServerRole::User, 0)
          .await
          .unwrap();
      let world = repo.create_world_owned("test", uid, 0).await.unwrap();

      let state = AppState {
          repo: repo.clone(),
          config: Arc::new(cfg),
          setup_token: None,
          initialized: Arc::new(AtomicBool::new(true)),
          ws: shadowcat::ws::WsState::new(),
          upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
          uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
          auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
          write_barrier: Arc::new(tokio::sync::RwLock::new(())),
          preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
      };
      // ... unchanged from here: ws_state/app/listener/login/Harness construction ...
  ```
  (only the reordering — building `cfg` before `repo`, and `.with_modules_dir(cfg.modules_path())`
  — changes; every line after `let state = AppState { ... }` is untouched. `world` is unused by
  `with_modules_dir` itself and keeps its existing position after `repo`.)

- [ ] **Step 1:** write a failing integration test in
  `src/server/src/data/sqlite/tests/rows_and_validation.rs` (or the file `import_world`'s
  existing tests live in — confirm the exact sibling test module via `rg "async fn
  import_world" src/server/src/data/sqlite -l` then `rg "mod " ` on that directory's `tests/`)
  asserting: a bundle whose settings carry a `validators_enabled: true` entry and whose imported
  document's `system` a hand-built refusing validator rejects, refuses the whole import with
  `DataError::OpFailed` containing `"validator "`; a bundle whose imported document's `system`
  violates a GM-declared tier-2 schema present in the bundle's own `world-schemas` settings row
  is rejected with `DataError::SchemaViolation`, never `DataError::Validator`, even when the
  bundle also enables an ALWAYS-FAULTING validator for that `doc_type` — proving
  `sandbox::validate_document`'s structural pre-pass runs before `import_world`'s validator pass
  exactly as it does in `apply_intent`; `apply_command`'s own replay path (used
  nowhere in `import_world` — this is a negative-space assertion) is untouched. Then implement.
- [ ] **Step 2:** `cargo test --all` (background + log) PASS; both clippy invocations + fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(sandbox): validate import_world's bulk writes; wire modules_dir into production/test-server/harness repositories" -- src/server/src/data/sqlite/export_import.rs src/server/src/main.rs src/server/src/bin/test_server.rs src/server/test-support/`

---

### Task 8: `ServerMsg::Reject.detail` wire field

**Files:**
- Modify: `src/server/src/ws/protocol.rs`'s `ServerMsg::Reject` variant:
  ```rust
      Reject {
          /// The refused intent's correlation token.
          intent_id: Uuid,
          /// Why it was refused.
          reason: RejectReason,
          /// Player/GM-presentable detail text — populated for `DataError::OpFailed`/`Validator`
          /// refusals (≤ 512 bytes, control characters stripped at the source that produced the
          /// text — `sandbox::runtime::run_validator` for a validator refusal). Rendered by the
          /// client as a TEXT NODE only, never HTML.
          #[serde(default)]
          detail: Option<String>,
      },
  ```
- Modify: `src/server/src/ws/conn.rs`:
  - `reject_reason` becomes:
    ```rust
    /// Map a write-path error to the client-actionable reject category, plus an optional
    /// player-presentable detail string carried on `ServerMsg::Reject.detail`.
    fn reject_reason(e: &crate::data::DataError) -> (RejectReason, Option<String>) {
        use crate::data::DataError::*;
        match e {
            Forbidden => (RejectReason::Forbidden, None),
            Conflict(_) => (RejectReason::Conflict, None),
            OpFailed(m) => (RejectReason::Invalid, Some(m.clone())),
            Validator(fault) => {
                (RejectReason::Invalid, Some(format!("validator {} faulted", fault.module)))
            }
            _ => (RejectReason::Invalid, None),
        }
    }
    ```
  - The two `ServerMsg::Reject { intent_id, reason }` construction sites in `handle_socket`'s
    ingress loop (the `ops_target_message` early-reject and the `room.publish(...)` error arm)
    become:
    ```rust
                                    if crate::chat::ops_target_message(&ops) {
                                        let _ = etx
                                            .send(Egress::Frame(Arc::new(ServerMsg::Reject {
                                                intent_id,
                                                reason: RejectReason::Forbidden,
                                                detail: None,
                                            })))
                                            .await;
                                        continue;
                                    }
                                    match room.publish(repo.as_ref(), &ctx, ops, now_millis(), WriteOrigin::Client).await {
                                        Ok(_cmd) => {}
                                        Err(e) => {
                                            if let crate::data::DataError::Validator(fault) = &e {
                                                if fault.consecutive >= crate::sandbox::VALIDATOR_FAULT_LIMIT {
                                                    room.disable_faulting_validator(repo.as_ref(), &ctx, &fault.module).await;
                                                }
                                            }
                                            let (reason, detail) = reject_reason(&e);
                                            tracing::debug!(world = %world_id, %intent_id, ?reason, "intent rejected");
                                            let _ = etx
                                                .send(Egress::Frame(Arc::new(ServerMsg::Reject {
                                                    intent_id,
                                                    reason,
                                                    detail,
                                                })))
                                                .await;
                                        }
                                    }
    ```
    (`Room::disable_faulting_validator` and the `Repository::reset_validator_fault_streak` trait method
    it depends on are added in Task 9; this task's gate battery will not compile until Task 9
    lands — these two tasks are committed as a matched pair: implement Task 9's `Room`/
    `Repository` changes FIRST within this same task's working tree before running gates, then
    let Task 9's own commit step land that diff separately. If the dispatcher requires strictly
    one file-set per task, merge Tasks 8 and 9 into one coder dispatch — they are two halves of
    one wire. A successful `room.publish` needs no explicit fault-counter reset here: each
    module's own streak already reset the instant its own call inside `sandbox::validate_document`
    returned `Accept`/`Refuse`.)
- Modify: `src/server/src/ws/protocol/protocol_tests.rs` — the existing `ServerMsg::Reject { .. }`
  construction (`let m = ServerMsg::Reject { ... }`) gains `detail: None`.
- Modify: `src/client/core/src/wire.ts`:
  - The `type: "reject"` TS union member gains, after `reason`:
    ```ts
        /** Player/GM-presentable detail text, rendered as a TEXT NODE only. */
        detail: string | null;
    ```
  - The `z.object({ type: z.literal("reject"), intent_id: z.string(), reason:
    RejectReasonSchema, })` Zod schema gains `detail: z.string().nullish(),` — `.nullish()`
    accepts `undefined` (a server that has not yet been upgraded / an old cached frame) as well
    as `null`.
- Modify: `src/client/core/src/ws-client.ts`:
  - `WsClientHandlers.onReject?(intentId: string, reason: RejectReason): void;` becomes
    `onReject?(intentId: string, reason: RejectReason, detail: string | null): void;`.
  - The dispatch site: `this.safeEmit(() => this.opts.handlers.onReject?.(msg.intent_id,
    msg.reason))` becomes `this.safeEmit(() => this.opts.handlers.onReject?.(msg.intent_id,
    msg.reason, msg.detail ?? null))`.
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`:
  - `WorldSessionOpts.onReject?: (reason: RejectReason) => void;` becomes
    `onReject?: (reason: RejectReason, detail: string | null) => void;`.
  - The inner handler `onReject: (id, reason) => { this.#optimistic.reject(id);
    this.opts.onReject?.(reason); }` becomes `onReject: (id, reason, detail) => {
    this.#optimistic.reject(id); this.opts.onReject?.(reason, detail); }`.
- Modify: `src/client/shell/src/lib/worldSession.test.ts` line 278/280:
  ```ts
    push({ type: "reject", intent_id: intent.intent_id, reason: "forbidden", detail: null });

    await vi.waitFor(() => expect(onReject).toHaveBeenCalledExactlyOnceWith("forbidden", null));
  ```
  Add a NEW test in the same file, after this one:
  ```ts
  test("a reject frame with detail passes it through to onReject", async () => {
    let push!: (frame: unknown) => void;
    const connect: Connect = (handlers) => {
      push = (frame) => handlers.onMessage(JSON.stringify(frame));
      queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
      return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
    };
    const onReject = vi.fn();
    const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger, onReject });
    await session.enter("w1");

    push({ type: "reject", intent_id: "00000000-0000-0000-0000-000000000001", reason: "invalid", detail: "validator example-module: hp must be non-negative" });

    await vi.waitFor(() =>
      expect(onReject).toHaveBeenCalledWith("invalid", "validator example-module: hp must be non-negative"),
    );
  });
  ```
- Modify: `src/client/shell/src/App.svelte` line 187:
  ```svelte
        onReject: (reason, detail) =>
          notifications.push(
            "warning",
            detail ? `${t(`intent.rejected.${reason}`)} ${detail}` : t(`intent.rejected.${reason}`),
          ),
  ```
- Modify: `src/client/shell/src/App.test.ts` — add, top of file after the existing imports,
  `import { activeNotifications } from "@shadowcat/ui-kit";`, then append a new test using the
  file's own established `vi.spyOn(WorldSession.prototype, "enter")` pattern PLUS a constructor
  spy to capture `App.svelte`'s `onReject` option directly (no WS round-trip needed — the
  callback under test is the small arrow function `App.svelte` passes to `new WorldSession({...})`):
  ```ts
  test("a reject's detail renders as a literal text node, never markup", async () => {
    vi.spyOn(WorldSession.prototype, "enter").mockResolvedValue(undefined);
    let capturedOnReject: ((reason: string, detail: string | null) => void) | undefined;
    const ctorSpy = vi
      .spyOn(WorldSession.prototype, "constructor" as never)
      .mockImplementation(function (this: unknown, opts: { onReject?: typeof capturedOnReject }) {
        capturedOnReject = opts.onReject;
      } as never);
    getSessionState().lastWorld = "w1";
    render(App);
    await vi.waitFor(() => expect(capturedOnReject).toBeDefined());

    capturedOnReject?.("invalid", "<b>evil</b>");

    const notice = activeNotifications().at(-1);
    expect(notice?.message).toContain("<b>evil</b>");
    ctorSpy.mockRestore();
  });
  ```
  (If spying on `WorldSession.prototype.constructor` proves awkward under this project's
  TypeScript/vitest configuration — constructors are not always interceptable this way across
  transpilation targets — the equivalent, always-workable alternative is `vi.mock("./lib/
  worldSession.svelte", ...)` at the top of the file replacing the real `WorldSession` class with
  a stub whose constructor records `opts` on a module-level variable this test reads; mirror
  whichever of the two the file's OTHER tests already lean toward once read in full — the
  assertion shape (`notice?.message` containing the literal `"<b>evil</b>"` string) is the
  load-bearing part of this test, not the capture mechanism.)
- Modify: `src/client/ui-kit/src/locales/en.ts` — no new keys needed; `detail` is appended as
  raw text, not a new i18n key.

- [ ] **Step 1:** write every test above FIRST (Rust `protocol_tests.rs` update,
  `worldSession.test.ts`'s two cases, `App.test.ts`'s new case); implement.
- [ ] **Step 2:** `cargo test --all` (background + log) PASS (this task alone will NOT compile
  until Task 9's `Room::disable_faulting_validator` and `Repository::reset_validator_fault_streak`
  exist — see the note in the Files section; if executed strictly in order, implement Task 9's
  `room.rs`/`repository.rs`/`sqlite.rs` changes as part of this task's working tree before
  running gates, and let Task 9's commit step below land that diff under its own message).
  `pnpm -r typecheck`, `pnpm --filter @shadowcat/core test`,
  `pnpm --filter @shadowcat/shell test`, both clippy invocations, `cargo fmt --check` PASS.
- [ ] **Step 3:** `git commit -m "feat(ws): Reject.detail carries a player-presentable refusal reason" -- src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/server/src/ws/protocol/ src/client/core/src/wire.ts src/client/core/src/ws-client.ts src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/worldSession.test.ts src/client/shell/src/App.svelte src/client/shell/src/App.test.ts`

---

### Task 9: `Room::disable_faulting_validator` — the auto-disable action

The consecutive-fault counter itself lives on `sandbox::registry::ValidatorRegistry` (Task 2) and
is maintained entirely by `sandbox::validate_document`. This task adds only the ACTION a caller
takes once a streak crosses `sandbox::VALIDATOR_FAULT_LIMIT`, plus the `Repository` trait method
`Room` needs to reset that streak afterward (it cannot reach `sandbox::registry` directly — `Room`
depends only on the `Repository` trait, never on `data`'s concrete types).

**Files:**
- Modify: `src/server/src/data/repository.rs` — add, after `set_world_enabled_modules`:
  ```rust
      /// Resets `module`'s consecutive sandbox-validator fault counter for `world` to zero —
      /// called by `Room::disable_faulting_validator` once it finishes disabling a persistently
      /// faulting module, so a future re-enable starts the streak at zero. A cheap, synchronous,
      /// in-memory operation: a repository never wired to a `modules_dir` has no counter to
      /// reset, so this is a no-op for it.
      ///
      /// # Examples
      ///
      /// ```
      /// # #[tokio::main]
      /// # async fn main() -> Result<(), shadowcat::data::DataError> {
      /// use shadowcat::data::repository::Repository;
      /// use shadowcat::data::sqlite::SqliteRepository;
      /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
      /// repo.reset_validator_fault_streak(uuid::Uuid::nil(), "mock-module").await;
      /// # Ok(())
      /// # }
      /// ```
      async fn reset_validator_fault_streak(&self, world: Uuid, module: &str);
  ```
- Modify: `src/server/src/data/sqlite.rs`'s `impl Repository for SqliteRepository` — add, after
  `set_world_enabled_modules`:
  ```rust
      async fn reset_validator_fault_streak(&self, world: Uuid, module: &str) {
          self.validator_registry_cache.reset_faults(world, module);
      }
  ```
- Modify: `src/server/src/ws/room/tests/mod.rs`'s `DeleteMidHydration` — add, after
  `set_world_enabled_modules`:
  ```rust
      async fn reset_validator_fault_streak(&self, world: Uuid, module: &str) {
          self.inner.reset_validator_fault_streak(world, module).await;
      }
  ```
- Modify: `src/server/src/ws/room.rs` — add, after `commit_combat`:
  ```rust
  impl Room {
      /// Disables `module`'s `validators_enabled` flag for this world (its own transaction, via
      /// `set_world_enabled_modules`), posts a GM-only chat notice (`build_message_doc` +
      /// `Audience::GmOnly`, committed under `WriteOrigin::ConfigSeed` — a second, independent
      /// `commit_ops_locked` call under a freshly-acquired `publish_guard`), and resets
      /// `module`'s consecutive-fault counter (`Repository::reset_validator_fault_streak`) so a future
      /// re-enable starts clean. Performs NO threshold check itself — the CALLER (`ws::conn`'s
      /// ingress loop) decides when `crate::sandbox::VALIDATOR_FAULT_LIMIT` is reached and calls
      /// this unconditionally at that point. The disable write runs even if the notice fails: the
      /// safety property does not depend on the notice succeeding. Never called while a caller's
      /// own `publish_guard` is held.
      pub(crate) async fn disable_faulting_validator(
          &self,
          repo: &dyn Repository,
          ctx: &PermissionContext,
          module: &str,
      ) {
          if let Ok(mut entries) = repo.world_enabled_modules(self.world_id).await {
              if let Some(entry) = entries.iter_mut().find(|e| e.id == module) {
                  entry.validators_enabled = false;
                  if let Err(e) = repo.set_world_enabled_modules(self.world_id, &entries).await {
                      tracing::warn!(world = %self.world_id, module, error = %e, "validator auto-disable write failed");
                  }
              }
          }
          let doc = crate::chat::build_message_doc(
              self.world_id,
              ctx.user_id,
              crate::chat::MessageDraft {
                  channel: "sandbox".to_string(),
                  actor_owner: None,
                  audience: crate::chat::Audience::GmOnly,
                  kind: crate::chat::MessageKind::System,
                  content: vec![crate::chat::Segment::Text {
                      text: format!(
                          "Sandboxed validator '{module}' faulted {} times in a row and has been disabled for this world.",
                          crate::sandbox::VALIDATOR_FAULT_LIMIT
                      ),
                  }],
                  source: None,
              },
              crate::ws::time::now_millis(),
          );
          let _guard = self.publish_guard.lock().await;
          if let Err(e) = self
              .commit_ops_locked(
                  repo,
                  ctx,
                  vec![Operation::Create { doc }],
                  crate::ws::time::now_millis(),
                  WriteOrigin::ConfigSeed,
              )
              .await
          {
              tracing::warn!(world = %self.world_id, module, error = %e, "validator auto-disable notice failed");
          }
          repo.reset_validator_fault_streak(self.world_id, module).await;
      }
  }
  ```
  (this `impl Room` block is a SECOND `impl Room` in the file — Rust allows multiple `impl`
  blocks for one type; place it directly after the existing `impl Room { ... }` block closes,
  not nested inside it, to avoid a large diff against the existing block's body.)

- [ ] **Step 1:** write a failing unit test in `src/server/src/ws/room/tests/mod.rs`:
  `disable_faulting_validator` called once — on a world enabled+opted into a module — flips that
  module's `validators_enabled` to `false` and leaves every other enabled module untouched; a
  `GmOnly` message document exists in the room's world afterward. Observe the counter reset
  WITHOUT a dead accessor: wire a real `SqliteRepository` to a temp `modules_dir` containing a
  WAT-compiled validator missing its `validate` export (every call faults `FaultKind::BadAbi`,
  the cheapest deterministic fault), enable it for the world, drive
  `SqliteRepository::apply_intent` against it four times (asserting `DataError::Validator` each
  time — this method performs no threshold check of its own, so the test never needs a real
  streak of 5 to exercise it), call `disable_faulting_validator` directly, re-enable the module,
  then drive `apply_intent` once more and assert the returned `DataError::Validator`'s
  `consecutive` reads `1` — proving the reset actually happened, not merely that the flag was
  cleared. Then implement.
- [ ] **Step 2:** `cargo test --all` (background + log) PASS (this pairs with Task 8's
  `ws/conn.rs` call sites — run the FULL suite, not just `ws::room`, to confirm both compile
  together); both clippy invocations + fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(sandbox): Room::disable_faulting_validator auto-disables a persistently faulting module" -- src/server/src/ws/room.rs src/server/src/ws/room/ src/server/src/data/repository.rs src/server/src/data/sqlite.rs`

---

### Task 10: `examples/validator-rust/` + `tests/sandbox.rs` + CI `wasm32-unknown-unknown`

**Files:**
- Create: `examples/validator-rust/Cargo.toml`:
  ```toml
  # Standalone workspace root: this crate is NOT a member of the repo's root Cargo workspace
  # (`Cargo.toml`'s `[workspace] members` does not list it) — an empty `[workspace]` here stops
  # Cargo from complaining "current package believes it's in a workspace when it's not listed as
  # a member" when built directly from this directory, which is how both the guide and
  # `tests/sandbox.rs` build it.
  [workspace]

  [package]
  name = "shadowcat-example-validator"
  version = "0.1.0"
  edition = "2021"

  [lib]
  crate-type = ["cdylib"]

  [profile.release]
  panic = "abort"
  opt-level = "z"
  lto = true
  ```
- Create: `examples/validator-rust/src/lib.rs`:
  ```rust
  //! Example sandboxed validator: refuses an `actor` whose `system.hp` is negative. `no_std`,
  //! no dependencies (a hand-rolled minimal JSON scan — no `serde-json-core`), no build script.
  //! Built directly with `cargo build --target wasm32-unknown-unknown --release`: this crate sits
  //! outside the repo's root Cargo workspace, so no workspace-level build script or feature runs
  //! against it.
  #![no_std]

  use core::panic::PanicInfo;

  /// A bump allocator over a fixed static arena — the whole guest ABI's `alloc` needs no
  /// deallocation (one call per validation, one validation per fresh `Store`).
  const ARENA_SIZE: usize = 64 * 1024;
  static mut ARENA: [u8; ARENA_SIZE] = [0; ARENA_SIZE];
  static mut NEXT: usize = 0;

  /// Reserves `len` bytes from the static arena and returns their offset, or `-1` if the arena is
  /// exhausted (the host treats any negative return as `FaultKind::BadPointer`).
  #[no_mangle]
  pub extern "C" fn alloc(len: i32) -> i32 {
      // SAFETY: single-threaded WASM guest, one call per Store per validation — no concurrent
      // access to `NEXT`/`ARENA` is possible.
      unsafe {
          let len = len as usize;
          if NEXT + len > ARENA_SIZE {
              return -1;
          }
          let ptr = NEXT;
          NEXT += len;
          ptr as i32
      }
  }

  /// Reads the `ValidatorInput` JSON at `(ptr, len)`, hand-scans for `"hp":<number>` inside the
  /// top-level `system` object, and refuses when that number is negative. Any other document
  /// (no `hp` key, or `hp >= 0`) is accepted. This is intentionally a minimal, forgiving scan —
  /// not a general JSON parser — matching the guide's stated scope.
  #[no_mangle]
  pub extern "C" fn validate(ptr: i32, len: i32) -> i32 {
      let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
      match find_hp(bytes) {
          Some(hp) if hp < 0 => 1,
          _ => 0,
      }
  }

  /// Byte-scans for the literal substring `"hp":` and parses the signed integer that follows
  /// (optional leading `-`, then ASCII digits, stopping at the first non-digit). Returns `None`
  /// if the key is absent or the following text is not a recognizable integer.
  fn find_hp(bytes: &[u8]) -> Option<i64> {
      const NEEDLE: &[u8] = b"\"hp\":";
      let pos = bytes
          .windows(NEEDLE.len())
          .position(|w| w == NEEDLE)?
          + NEEDLE.len();
      let mut i = pos;
      let negative = bytes.get(i) == Some(&b'-');
      if negative {
          i += 1;
      }
      let start = i;
      while bytes.get(i).is_some_and(u8::is_ascii_digit) {
          i += 1;
      }
      if i == start {
          return None;
      }
      let mut value: i64 = 0;
      for &b in &bytes[start..i] {
          value = value * 10 + i64::from(b - b'0');
      }
      Some(if negative { -value } else { value })
  }

  /// Static empty reason for the one refusal case this validator authors.
  static REASON: &[u8] = b"system.hp must not be negative";

  /// The refusal reason's address, for the host's `reason_ptr`/`reason_len` read after a
  /// non-zero `validate` return.
  #[no_mangle]
  pub extern "C" fn reason_ptr() -> i32 {
      REASON.as_ptr() as i32
  }

  /// The refusal reason's byte length.
  #[no_mangle]
  pub extern "C" fn reason_len() -> i32 {
      REASON.len() as i32
  }

  #[panic_handler]
  fn panic(_info: &PanicInfo) -> ! {
      loop {}
  }
  ```
- Create: `examples/validator-rust/module.json` (a minimal, valid module manifest declaring this
  validator, for the guide/integration test to install verbatim):
  ```json
  {
    "id": "example-validator",
    "version": "1.0.0",
    "dependencies": {},
    "engines": { "shadowcat": "*" },
    "validators": [{ "docType": "actor", "wasm": "validator.wasm" }]
  }
  ```
- Create: `src/server/tests/sandbox.rs`:
  ```rust
  //! End-to-end sandbox validator lifecycle: builds `examples/validator-rust/` for real (never
  //! skips — a missing `wasm32-unknown-unknown` target fails loudly, naming the fix), installs it
  //! into a temp modules dir, has the GM enable the module and opt into validators, and asserts a
  //! negative-`hp` actor Create is refused with the reason while a non-negative one is accepted.
  use shadowcat_test_support as common;
  use std::path::PathBuf;

  /// The example crate's own directory, resolved relative to this workspace member's manifest
  /// dir (`CARGO_MANIFEST_DIR` is `src/server/`), never the process's current directory.
  fn example_dir() -> PathBuf {
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
          .join("..")
          .join("..")
          .join("examples")
          .join("validator-rust")
  }

  /// Builds `examples/validator-rust/` for `wasm32-unknown-unknown` in release mode, returning
  /// the built `.wasm` bytes. Fails loudly (never skips) when the target is missing, naming the
  /// exact `rustup` command to run.
  fn build_example_wasm() -> Vec<u8> {
      let dir = example_dir();
      let status = std::process::Command::new("cargo")
          .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
          .current_dir(&dir)
          .status()
          .expect("cargo invocation itself must succeed (cargo must be on PATH)");
      assert!(
          status.success(),
          "building examples/validator-rust for wasm32-unknown-unknown failed — if this is a \
           missing-target error, run `rustup target add wasm32-unknown-unknown` and retry; this \
           test NEVER skips on a missing target, per the M28 spec's §5"
      );
      let wasm_path = dir
          .join("target")
          .join("wasm32-unknown-unknown")
          .join("release")
          .join("shadowcat_example_validator.wasm");
      std::fs::read(&wasm_path)
          .unwrap_or_else(|e| panic!("built wasm not found at {}: {e}", wasm_path.display()))
  }

  /// Copies the example's `module.json` (rewriting its `wasm` path to the flat filename this
  /// installs it under) plus the freshly-built `.wasm` into `<modules_dir>/example-validator/`.
  fn install_example(modules_dir: &std::path::Path) {
      let dest = modules_dir.join("example-validator");
      std::fs::create_dir_all(&dest).unwrap();
      let manifest = std::fs::read_to_string(example_dir().join("module.json")).unwrap();
      std::fs::write(dest.join("module.json"), manifest).unwrap();
      std::fs::write(dest.join("validator.wasm"), build_example_wasm()).unwrap();
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_negative_hp_actor_create_is_refused_a_non_negative_one_is_accepted() {
      let modules_dir = std::env::temp_dir().join(format!("shadowcat-sandbox-e2e-{}", uuid::Uuid::new_v4()));
      std::fs::create_dir_all(&modules_dir).unwrap();
      install_example(&modules_dir);

      let h = common::spawn_with(|cfg| {
          cfg.modules_dir = Some(modules_dir.to_string_lossy().into_owned());
      })
      .await;

      // GM enables the module and opts into validators.
      let entries = serde_json::json!([{ "id": "example-validator", "validators_enabled": true }]);
      let res = h
          .client
          .put(format!("http://{}/api/worlds/{}/enabled-modules", h.addr, h.world))
          .json(&entries)
          .send()
          .await
          .unwrap();
      assert_eq!(res.status(), 204, "enable request failed: {:?}", res.text().await);

      let mut ws = h.connect().await;
      common::drain_until_type(&mut ws, "welcome").await;

      // A negative-hp actor create is refused.
      use tokio_tungstenite::tungstenite::Message;
      let refused_doc = serde_json::json!({
          "op": "create",
          "doc": {
              "id": uuid::Uuid::new_v4(),
              "scope": { "kind": "world", "world_id": h.world },
              "doc_type": "actor",
              "schema_version": 1,
              "system": { "hp": -1 },
              "created_at": 0,
              "updated_at": 0,
          }
      });
      let intent_id = uuid::Uuid::new_v4();
      ws.send(Message::Text(
          serde_json::json!({ "type": "intent", "intent_id": intent_id, "ops": [refused_doc] })
              .to_string()
              .into(),
      ))
      .await
      .unwrap();
      let reject = common::drain_until_type(&mut ws, "reject").await;
      assert_eq!(reject["intent_id"], intent_id.to_string());
      assert!(
          reject["detail"]
              .as_str()
              .is_some_and(|d| d.contains("hp must not be negative")),
          "unexpected reject frame: {reject:?}"
      );

      // A non-negative-hp actor create is accepted.
      let accepted_doc = serde_json::json!({
          "op": "create",
          "doc": {
              "id": uuid::Uuid::new_v4(),
              "scope": { "kind": "world", "world_id": h.world },
              "doc_type": "actor",
              "schema_version": 1,
              "system": { "hp": 3 },
              "created_at": 0,
              "updated_at": 0,
          }
      });
      let intent_id_2 = uuid::Uuid::new_v4();
      ws.send(Message::Text(
          serde_json::json!({ "type": "intent", "intent_id": intent_id_2, "ops": [accepted_doc] })
              .to_string()
              .into(),
      ))
      .await
      .unwrap();
      let event = common::drain_until_event(&mut ws).await;
      assert_eq!(event["command"]["ops"][0]["doc"]["system"]["hp"], 3);
  }
  ```
- Modify: `.github/workflows/ci.yml`:
  - Line 28-30 (`rust` job's toolchain step):
    ```yaml
        - uses: dtolnay/rust-toolchain@stable
          with:
            components: rustfmt, clippy
            targets: wasm32-unknown-unknown
    ```
  - Line 179 (`docs` job's toolchain step):
    ```yaml
        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: wasm32-unknown-unknown
    ```
- Modify: `docs/site/guides/hosting.md` line 14:
  ```markdown
  **From source** (needs Rust stable with the `wasm32-unknown-unknown` target —
  `rustup target add wasm32-unknown-unknown` — Node 22, pnpm 9):
  ```

- [ ] **Step 1:** create the example crate; run `cargo build --target wasm32-unknown-unknown
  --release --manifest-path examples/validator-rust/Cargo.toml` locally (installing the target
  first via `rustup target add wasm32-unknown-unknown` if this machine lacks it) to confirm it
  actually compiles to a valid `cdylib` before writing the integration test against it.
- [ ] **Step 2:** write `tests/sandbox.rs`; `cargo test --all --manifest-path
  src/server/Cargo.toml -- sandbox` (background + log) PASS.
- [ ] **Step 3:** apply the CI + hosting.md edits.
- [ ] **Step 4:** `cargo fmt --check --manifest-path examples/validator-rust/Cargo.toml`
  (this crate has its own `[workspace]`, so fmt/clippy run against it independently —
  `cargo clippy --manifest-path examples/validator-rust/Cargo.toml --target
  wasm32-unknown-unknown -- -D warnings` PASS).
- [ ] **Step 5:** `git commit -m "test(sandbox): example validator crate + end-to-end integration test; wasm32 target on CI" -- examples/validator-rust/ src/server/tests/sandbox.rs .github/workflows/ci.yml docs/site/guides/hosting.md`

---

### Task 11: threat-model doc, creating-a-validator guide, ARCHITECTURE/PLAN updates

**Files:**
- Create: `docs/design/sandboxed-validators.md`:
  ```markdown
  # Sandboxed third-party validators — threat model

  Opt-in, per-world, per-module server-side validators over the `system` band only, running
  `wasm32-unknown-unknown` code inside `wasmi` (a pure-Rust interpreter, no JIT). Never the
  default path: a GM must install a module declaring validators AND explicitly opt this world
  into running them (`WorldModuleEntry.validators_enabled`).

  | Threat | Mitigation |
  |---|---|
  | CPU exhaustion (infinite loop) | fuel cap per call; fault counter auto-disables after 5 |
  | Slow-but-under-fuel validator throttling every world (the write pool is a single connection server-wide) | validators run OUTSIDE the write transaction against a read-only pre-image; a 50 ms wall-clock budget per call is a fault; 5 faults auto-disable |
  | Many simultaneous intents each running a validator | bounded by the single-writer pool: one intent reaches the write path at a time per server, so at most one validator per in-flight intent; stated here so a future pool change re-examines it |
  | Memory exhaustion | `StoreLimits` 16 MiB; module ≤ 4 MiB; input ≤ 1 MiB |
  | Escaping the sandbox | `wasmi` is a pure-Rust interpreter with no JIT and no host imports beyond `log`; no WASI |
  | Data exfiltration | no network/filesystem/clock imports; the only output channel is the ≤ 512-byte reason, visible to the writer who already holds the data |
  | Denial of writes to the table | GM opt-in per world per module; the GM can turn it off; auto-disable on faults; the GM's own writes are subject to it too (a validator that refuses everything is visible on the first write) |
  | Malicious reason text | carried on `ServerMsg::Reject.detail`, rendered as a text node only; length + control-char stripping |
  | Supply chain (a validator swapped on disk) | out of scope here — a future signing/SRI mechanism; this doc records the gap rather than papering over it |
  ```
- Create: `docs/site/guides/creating-a-validator.md`:
  ```markdown
  # Creating a validator

  A validator is a `wasm32-unknown-unknown` module a GM opts a world into running, server-side,
  over ONE document type's `system` band. Unlike a module's own client code (untrusted-but-
  admin-installed, see [Creating a module](/guides/creating-a-module)), a validator runs inside a
  sandbox: fuel-metered, memory-capped, and able to do exactly one thing — refuse a write with a
  reason. It cannot mutate, read other documents, observe time, or reach the network.

  Every code sample on this page is imported from `examples/validator-rust/` in the Shadowcat
  repository, which `src/server/tests/sandbox.rs` builds and runs on every push.

  ## Declaring a validator

  Add a `validators` array to your module's `module.json`:

  ```jsonc
  "validators": [{ "docType": "actor", "wasm": "validator.wasm" }]
  ```

  `wasm` is a path relative to your module's own install folder; a path escaping that folder is
  refused at scan time.

  ## The guest ABI

  `wasm32-unknown-unknown`, no WASI, no imports except `env.log(ptr: i32, len: i32)` (debug-level
  tracing, ≤ 1 KiB per call, ≤ 16 calls per validation — the 17th+ call is silently ignored).
  Required exports:

  ```
  memory                                // your linear memory
  alloc(len: i32) -> i32                // the host asks you for a buffer to write the input into
  validate(ptr: i32, len: i32) -> i32   // 0 = accept; non-zero = refuse
  reason_ptr() -> i32                   // after a non-zero validate return
  reason_len() -> i32
  ```

  The host writes a UTF-8 JSON object at the pointer `alloc` returns:

  ```jsonc
  {
    "docType": "actor",
    "op": "create",
    "system": { "hp": -1 },
    "prior": null,
    "name": "Goblin",
    "worldId": "…",
    "moduleId": "example-validator"
  }
  ```

  Nothing else reaches you: no `engine` band, no permissions, no other documents, no clock, no
  randomness. Your validator is a pure function of this input.

  ## Limits

  | Limit | Value |
  |---|---|
  | Module size | 4 MiB |
  | Input size | 1 MiB |
  | Guest memory | 16 MiB |
  | Wall clock per call (measured, post-return) | 50 ms — a slower call still runs to completion, but counts as a fault |
  | Wall clock hang guard (never-returning call) | 250 ms — the call is abandoned and counts as a fault |
  | Consecutive faults before auto-disable | 5 |

  ## Building

  No build script; a raw `cargo build` invocation:

  ```bash
  cargo build --target wasm32-unknown-unknown --release
  ```

  `examples/validator-rust/` is `no_std`, dependency-free, with a tiny bump allocator — see its
  `src/lib.rs` for the complete `alloc`/`validate`/`reason_ptr`/`reason_len` implementation, which
  refuses an `actor` whose `system.hp` is negative.

  ## Opting a world in

  A GM enables your module, then — only once it declares at least one validator — a second
  toggle, "Run sandboxed validators", appears in Settings → Installed modules. Both must be on;
  disabling the module also stops its validators regardless of the toggle's own state.

  See [the threat model](../../design/sandboxed-validators.md) for what the sandbox does and does
  not protect against.
  ```
- Modify: `docs/site/guides/creating-a-module.md` — after the existing "Modules are admin-trusted.
  There is no sandbox." bullet, add:
  ```markdown
  - **A module's client code is still fully trusted — this has not changed.** A module MAY
    additionally declare sandboxed, opt-in, SERVER-SIDE validators (a completely separate
    mechanism, `wasm32-unknown-unknown` code judging only the `system` band of one document
    type) — see [Creating a validator](creating-a-validator).
  ```
- Modify: `docs/site/.vitepress/config.mts` — add, after the `"Creating a system"` sidebar entry
  (line 27): `{ text: "Creating a validator", link: "/guides/creating-a-validator" },`.
- Modify: `docs/design/ARCHITECTURE.md`:
  - §3 table — add, after the `Build tooling` row (line 59):
    ```markdown
    | Sandboxed validators | wasmi (pure-Rust interpreter) | MIT/Apache-2.0 | Vendor | Third-party server-side WASM code, opt-in per world per module, fuel/memory/instance-limited; no JIT, no host imports beyond a rate-limited debug log — see `docs/design/sandboxed-validators.md`. |
    ```
  - §4 table — DELETE the row `| Server-side untrusted execution (sandbox) | ... |` (line 76):
    this item is no longer deferred, it is built.
  - §5 — rewrite the Deno bullet (line 85) from
    `- **Deno** — a ~100 MB V8 second runtime undercuts the single binary; its --allow-* model is
    a weak sandbox. Engine grammars are evaluated natively in Rust; no JS runtime is needed
    server-side.` to:
    ```markdown
    - **Deno / wasmtime / extism for the sandboxed-validator runtime** — Deno is a ~100 MB V8
      second runtime that undercuts the single binary and whose `--allow-*` model is a weak
      sandbox; `wasmtime`'s Cranelift JIT adds 15–20 MiB and a JIT attack surface a validator run
      once per write over kilobytes of data does not need. `wasmi` (a pure-Rust interpreter, no
      JIT, ~1 MiB) is the sandbox this project actually ships — see
      `docs/design/sandboxed-validators.md`. Engine grammars themselves are still evaluated
      natively in Rust; no JS runtime is needed server-side for anything the engine defines.
    ```
- Modify: `docs/PLAN.md` — DELETE the two-sentence paragraph at lines 61-63 (`Also parked for
  Phase 3 from Phase 1: capability Phase 3 — opt-in **sandboxed** server-side validators running
  third-party *code* (its own threat model; never the default path). Server-side evaluation of
  the engine's own grammars is not this item — it shipped in M14c-1.`) — this specific item is no
  longer parked, it is delivered; the surrounding "Phase 3 — Atmosphere" heading and its
  audio/VFX/etc. paragraph (line 59) are UNTOUCHED (M28 is not the last Phase-3 milestone to
  merge — only the last milestone flips that heading, per the master spec's §3).

- [ ] **Step 1:** write every doc file above.
- [ ] **Step 2:** `pnpm docs:check-examples` (the guide's fenced `jsonc`/`bash` blocks are
  prose-only, not imported TS, so this gate is a no-op for this task's new files — confirm it
  still passes overall), `pnpm build:all` (background + log; regenerates `dist-docs`), the two
  `cargo doc` gates from the `docs` CI job locally if time permits (`pnpm docs:check-rust-examples`
  needs the nightly toolchain — run it if available, otherwise defer to the LAST task's full
  battery run).
- [ ] **Step 3:** `git commit -m "docs(sandbox): threat model, creating-a-validator guide, ARCHITECTURE/PLAN updates" -- docs/`

---

### Task 12: skill update (new `shadowcat-codebase-sandbox`, `module-toolchain`/`documents-permissions` updates), HISTORY entry, binary-size measurement

**Files (plugin checkout, edited WITHOUT committing there per the dispatcher's own gate — this
task's coder edits the files, the dispatcher reviews and commits in the plugin repo separately):**
- Create: `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-sandbox/SKILL.md` —
  fixed shape (Purpose / Key files & seams / Hard invariants / Gotchas / Pointers):
  ```markdown
  ---
  description: Sandboxed third-party server-side validators (wasmi) over the system band
  ---

  # shadowcat-codebase-sandbox

  ## Purpose
  Opt-in, per-world, per-module server-side validators: `wasm32-unknown-unknown` code running
  inside `wasmi`, judging one document type's `system` band, able only to refuse a write.

  ## Key files & seams
  - `src/server/src/sandbox/mod.rs` — `ValidatorVerdict`, `ValidatorFault`, `FaultKind`,
    `ValidatorInput`, `VALIDATOR_FAULT_LIMIT`, `validate_structural` (Phase 1's own pure
    structural chain, re-run here before any validator), `validate_document` (the embedded-tree
    walk AND the fault counter's own increment/reset calls; the walk mirrors
    `validation::validate_system_schema_tree`'s recursion).
  - `src/server/src/sandbox/runtime.rs` — the wasmi host: `CompiledValidator`, `run_validator`
    (fuel/memory/instance limits, the `env.log` import, the 50ms `TooSlow`/250ms `Hung`
    wall-clock pair via `run_validator_with_budgets`).
  - `src/server/src/sandbox/registry.rs` — `ValidatorRegistry`/`ValidatorRegistryCache` (compiled-
    once cache, mtime-invalidated like `crate::modules::ModuleScanCache`; also the home of the
    per-(world, module) consecutive-fault `DashMap`, surviving a rescan via
    `ValidatorRegistryCache`'s own persistent copy).
  - `src/server/src/modules.rs` — `ValidatorDecl`, `InstalledModule.validators`,
    `WorldModuleEntry` (the per-world `id` + `validators_enabled` enablement record,
    `WorldModuleEntry::parse_legacy_tolerant` for the pre-M28 bare-string-array back-compat read).
  - `src/server/src/data/sqlite.rs` — `SqliteRepository::apply_intent`'s pre-transaction
    validator pass (BEFORE `self.pool.begin()`, against a read-only pool pre-image);
    `merge_update_document` (shared by Phase 2's real merge and the validator pre-image build);
    `reset_validator_fault_streak` (delegates to `ValidatorRegistryCache::reset_faults`).
  - `src/server/src/data/sqlite/export_import.rs` — `import_world`'s in-loop validator pass
    (inside its own already-exclusive transaction — no throttling concern there, unlike
    `apply_intent`).
  - `src/server/src/ws/room.rs` — `Room::disable_faulting_validator` (the disable write + GM-only
    chat notice + counter reset, acted on once a caller decides
    `crate::sandbox::VALIDATOR_FAULT_LIMIT` is reached).
  - `src/server/src/ws/protocol.rs` — `ServerMsg::Reject.detail`.
  - `examples/validator-rust/` — the reference `no_std` validator; `src/server/tests/sandbox.rs`
    builds it for real on every `cargo test --all`.

  ## Hard invariants
  - Validators run over `system` ONLY, never `engine` — the engine band is already
    server-validated territory (ARCHITECTURE §2 invariant 6); a validator seeing it would blur
    that boundary.
  - `apply_command` (the trusted undo/replay substrate) NEVER invokes a validator — only
    `apply_intent` and `import_world`.
  - `apply_intent`'s validator pass runs OUTSIDE the write transaction (the single-writer pool
    would otherwise throttle every hosted world on a slow validator); `import_world`'s does not
    need to, since that function already holds the writer exclusively for its whole duration.
  - `sandbox::validate_document` always re-runs Phase 1's own pure structural validators
    (`validate_system_size`/`validate_property_overrides`/`validate_engine_tree`/
    `validate_containment`/`validate_system_schema_tree`) against the document BEFORE consulting
    any validator, returning that error untouched on failure — a malformed submission never
    reaches, and never faults, a validator, in either `apply_intent` or `import_world`.
  - A validator can refuse; it cannot mutate the document, read others, observe time, or reach
    the network — enforced by giving it no imports beyond `env.log`, not by policy alone.
  - The consecutive-fault counter is COUNTED entirely inside `sandbox::validate_document`
    (per-(world, module), on `ValidatorRegistry`); `ws::conn` — never `sandbox` or `data` — is
    what ACTS on it, since only `ws` can reach `Room`.

  ## Gotchas
  - `SqliteRepository::modules_dir` defaults to `None` on every plain `::connect()` call —
    validators silently never run until `.with_modules_dir(..)` is called (production `main.rs`,
    `test_server.rs`, and `test-support::spawn_with` all do; a bespoke test harness that builds
    its own `SqliteRepository` without calling it will never see a validator fire, which usually
    means the test just needs that one call added, not a sandbox bug.
  - `WorldModuleEntry`'s wire shape changed from a bare `string[]` — a stored settings row from
    before M28 parses via `parse_legacy_tolerant`'s fallback, reading as `validators_enabled:
    false` for every id; do not "fix" an old dev DB by hand, the fallback already handles it.
  - A module's own `Refuse` verdict resets its fault streak exactly like `Accept` — only a
    `Fault` verdict is evidence the sandbox itself is broken; an authored refusal means the
    module ran correctly. `Room::disable_faulting_validator` performs no threshold check of its
    own and will disable a module on a single call regardless of its actual streak — the caller
    decides when `VALIDATOR_FAULT_LIMIT` is reached.

  ## Pointers
  - Threat model: `docs/design/sandboxed-validators.md`.
  - Author-facing guide: `docs/site/guides/creating-a-validator.md`.
  - Design spec: `docs/superpowers/specs/2026-09-11-m28-sandboxed-validators-design.md`.
  ```
  and add its glob to `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` map:
  `"sandbox": ["src/server/src/sandbox/", "examples/validator-rust/"]` (match the map's existing
  entry shape exactly — read one neighboring entry first to confirm key/value structure before
  editing) plus one absolute-path assertion line in
  `hooks/test-codebase-skill-reminder.sh` (e.g.
  `check "C:/Dev/Shadowcat/src/server/src/sandbox/mod.rs" "sandbox"` — match the script's
  existing `check` invocation shape for another subsystem exactly).
- Modify: `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-module-toolchain/SKILL.md`
  — add a short paragraph under its existing manifest-key coverage noting the new `validators`
  key and pointing to `shadowcat-codebase-sandbox` for the runtime; do not duplicate content
  already stated there.
- Modify: `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-documents-permissions/SKILL.md`
  — add a short paragraph noting `apply_intent`'s validator chokepoint placement (before the
  transaction, `system` band only) and pointing to `shadowcat-codebase-sandbox`.

- [ ] **Step 1:** write the new skill and the two updates above.
- [ ] **Step 2:** dispatch `shadowcat-codebase:shadowcat-spec-reviewer` (sonnet, effort high) on
  the plugin-checkout diff alone (a `git diff` run FROM inside
  `~/.claude/skills/shadowcat-codebase/`), confirming no omission/drift/broken pointer against
  this plan's actual delivered symbols. Apply any finding.
- [ ] **Step 3:** `node scripts/check-skill-symbol-refs-cli.mjs` (0 broken), `pnpm run
  test:scripts`, `node scripts/check-skill-api-refs-cli.mjs` (needs `pnpm build:all`'s
  `dist-docs`, already built in Task 11) — all run from THIS repo's root, against the plugin
  checkout.
- [ ] **Step 4:** measure the binary-size delta: `cargo build --release --manifest-path
  src/server/Cargo.toml` (background + log) BEFORE this milestone's dependency was added is no
  longer measurable retroactively — instead, record the CURRENT release binary size via `pnpm
  lint:binary-size`'s own reported number (it prints the measured size against the 60 MiB cap)
  and the `wasmi`/`wat` versions from Task 1, for the HISTORY entry below.
- [ ] **Step 5:** `docs/HISTORY.md` — append under `## Phase 3 — Atmosphere` (create the heading
  if this is the first Phase-3 entry to land, per master §3's "create the heading if absent"):
  ```markdown
  ### M28 · Sandboxed third-party validators ✅
  Branch `m28-sandbox`, cut from `main`, executed from the approved plan
  `docs/superpowers/plans/2026-09-11-m28-sandboxed-validators.md` (design:
  `docs/superpowers/specs/2026-09-11-m28-sandboxed-validators-design.md`). Delivered:
  - **`wasmi` <version> + `wat` <version>** (Task 1 verified the resolved crate's API against
    the spec's assumed names; corrections, if any, are recorded in the spec's §3). Release
    binary size after this milestone: <measured bytes> / 60 MiB cap.
  - **`src/server/src/sandbox/`**: `ValidatorVerdict`/`ValidatorFault`/`FaultKind`/`ValidatorInput`
    (`validate_document`'s embedded-tree walk, mirroring
    `validation::validate_system_schema_tree`'s recursion exactly, AND its own maintenance of the
    per-(world, module) fault counter), `runtime::run_validator` (fuel/memory/instance-limited
    per-call `wasmi` host, `env.log` the only import, a 50ms post-hoc `TooSlow` reclassification
    plus a 250ms `Hung` hang guard around the `spawn_blocking` join), `registry::ValidatorRegistry`
    (compiled-once, mtime-invalidated cache; also home of the per-(world, module) consecutive-
    fault `DashMap`, surviving a rescan via `ValidatorRegistryCache`).
  - **Manifest + discovery**: `module.json`'s `validators` key, `InstalledModule.validators`,
    the traversal guard shared with `http::module_routes::serve_module_file`.
  - **`WorldModuleEntry`** replaces the bare enabled-module-id `Vec<String>` (`Repository::
    world_enabled_modules`/`set_world_enabled_modules`, `GET`/`PUT
    /api/worlds/{world}/enabled-modules`, `module-rest.ts`, `ModuleManager.svelte`'s second
    "Run sandboxed validators" toggle) — a legacy string-array setting reads as every id
    `validators_enabled: false`.
  - **Placement**: `apply_intent`'s validator pass runs BEFORE the write transaction opens,
    against a read-only pre-image, relying on Phase 1's existing OCC check to catch a stale
    read; `import_world`'s runs inside its own already-exclusive transaction. Both chokepoints
    call the SAME `sandbox::validate_document`, which first re-runs Phase 1's own pure structural
    validators on the document and returns that error untouched on failure — a malformed
    submission never reaches, or counts against, a validator. Never `apply_command`.
  - **Fault policy**: `DataError::Validator(ValidatorFault)` (technical fault, carrying
    `module`/`kind`/`consecutive`) alongside the existing `DataError::OpFailed` (an authored
    refusal); `ServerMsg::Reject.detail` (new optional wire field) carries the reason to
    `App.svelte`'s toast as a text node. The consecutive-fault counter is COUNTED inside
    `sandbox::validate_document` (a `DashMap` on `sandbox::registry::ValidatorRegistry`, surviving
    a rescan via `ValidatorRegistryCache`) and ACTED ON by `ws::conn`, which calls
    `Room::disable_faulting_validator` at `sandbox::VALIDATOR_FAULT_LIMIT` (5) — it disables the
    module, posts a GM-only notice, and resets the streak via the new
    `Repository::reset_validator_fault_streak`.
  - **`examples/validator-rust/`**: a `no_std`, dependency-free reference validator (refuses a
    negative `actor.system.hp`); `src/server/tests/sandbox.rs` builds it for real on every
    `cargo test --all` and never skips on a missing `wasm32-unknown-unknown` target; the target
    was added to the three-OS `rust` CI job and the `docs` job.
  - **Docs**: `docs/design/sandboxed-validators.md` (threat model),
    `docs/site/guides/creating-a-validator.md`, a cross-link from `creating-a-module.md`,
    ARCHITECTURE.md §3/§4/§5 updated, `docs/PLAN.md`'s parked "capability Phase 3" paragraph
    removed.
  - **Skills**: new `shadowcat-codebase-sandbox`; `module-toolchain`/`documents-permissions`
    updated — reviewed by `shadowcat-codebase:shadowcat-spec-reviewer`, committed in the plugin
    repo.
  - **Recorded design decisions** (spec gaps the amendment left implicit, not
    re-interpretations of an explicit instruction): the fault counter lives on
    `sandbox::registry::ValidatorRegistry`/`ValidatorRegistryCache` rather than `Room`, since a
    compiled-module registry is rebuilt on every rescan and the counter must survive that
    rebuild; `ValidatorVerdict::Fault` and `DataError::Validator` share one `ValidatorFault`
    payload type rather than duplicating its three fields; a module's own `Refuse` resets its
    streak exactly like `Accept`, since only `Fault` is evidence of a technical break.
  ```
- [ ] **Step 6:** `git commit -m "docs: M28 HISTORY entry" -- docs/HISTORY.md` (this repo).
  Commit + push the plugin-checkout diff from INSIDE
  `~/.claude/skills/shadowcat-codebase/` (its own remote), per the dispatcher's skill-update gate
  — this step runs in a SEPARATE working tree/repo from every other step in this plan.

---

### Task 13: merge-forward integration (LAST task; dispatcher-gated)

Per the master spec's §5, M28 merges to `main` SECOND, immediately after M22. This task runs only
after the dispatcher confirms M22 is on `main`.

- [ ] **Step 1:** `git fetch origin && git merge origin/main` in the `m28-sandbox` worktree (a
  merge commit, never a rebase). Expected conflicts per master §3: `src/server/Cargo.toml`
  (M22 adds no server dependency, so this is likely conflict-free — M28's hunk is the FIRST
  Phase-3 addition, landing under its own `# Phase 3: M28` header); `docs/HISTORY.md`/
  `docs/PLAN.md` (M22's own entries, if it merged first as scheduled). M28 consumes no seam
  from any other Phase-3 milestone (master §2.6: "No other milestone consumes it"), so this
  merge wires up nothing new beyond conflict resolution.
- [ ] **Step 2:** re-run the FULL gate battery listed under "Global constraints" above,
  including `pnpm --filter @shadowcat/core test:e2e` and the release-build
  `pnpm lint:binary-size` (background + log every long command; read every log before claiming
  green).
- [ ] **Step 3:** buddy-check the WHOLE branch diff (`git diff main...m28-sandbox`) — spec
  reviewer + code reviewer, blind, dispatcher-pre-generated diff. Fix any finding in a new
  commit; re-run the affected gate subset.
- [ ] **Step 4:** `git rev-parse origin/main main` measured (dispatcher sequencing check);
  confirm the branch contains `origin/main`; `pnpm gate:push` receipt present for the branch
  HEAD; `git commit` the merge (trailer appended) if not already committed by Step 1; `git push`.
- [ ] **Step 5:** open the PR (`gh pr create`), wait for CI (`gh run watch`), merge after BOTH
  CI runs are green (`main` is branch-protected, `--auto` is off).
- [ ] **Step 6:** report to the dispatcher: STATUS, every commit, every gate line, the measured
  binary-size delta, the plugin-repo commit hash, the merged PR URL, and the three recorded
  design decisions (where the fault counter lives, the shared `ValidatorFault` payload type, a
  module's own `Refuse` resetting its streak like `Accept`) for the dispatcher to carry into the
  campaign's own final report.
