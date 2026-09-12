# M28 — Sandboxed third-party validators — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.6 the seam this milestone
> owns; §7 wasmi; §9 D8). This is PLAN.md's parked "capability Phase 3": opt-in sandboxed
> server-side validators running third-party CODE over the `system` band. It has its own
> threat model (§4) and is never the default path. ARCHITECTURE §2 invariant 6 stays intact:
> the ENGINE still never interprets `system` semantics — a module the GM opted in does, inside
> a box the engine controls.

## 1. Manifest + discovery (`modules.rs`, `manifest.ts`)

```jsonc
// module.json (appended key)
"validators": [ { "docType": "actor", "wasm": "validators/actor.wasm" } ]
```

- `InstalledModule.validators: Vec<ValidatorDecl { doc_type: String, wasm: PathBuf /* under
  the module folder; `..` refused by the same traversal guard `module_routes` uses */ }>`;
  `ModuleManifest.validators?: { docType: string; wasm: string }[]` mirrored in the client
  manifest schema (advisory display only — the client never runs one).
- At scan, each declared `.wasm` is read (≤ 4 MiB), validated and COMPILED once into a cached
  `wasmi::Module` (`sandbox::registry`); a failure logs a warning and marks the entry
  `load_error: Some(String)` (fail-open on discovery like manifests — one broken validator
  never hides the module), shown in `ModuleManager`.
- Per-world opt-in. Today the world's enablement record is a bare `Vec<String>` of module
  ids (`Repository::world_enabled_modules`/`set_world_enabled_modules`, stored through
  `set_setting` as a JSON string array, mirrored by `ModuleManager.svelte`'s `enabled:
  Set<string>` and its whole-set `setEnabledModules` PUT). This milestone changes that record
  to `Vec<WorldModuleEntry { id: String, validators_enabled: bool }>` — the `Repository`
  trait signatures, the HTTP wire type behind `setEnabledModules`, the client wrapper in
  `module-rest.ts` and `ModuleManager.svelte` all move together (a plain string array in a
  stored setting is read as `validators_enabled: false` for every id, so an existing dev
  DB keeps working). `ModuleManager.svelte` shows a second toggle "Run sandboxed
  validators" — GM-only, only for modules that declare any, with the load status beside it.
  A module disabled for the world never runs validators regardless of the flag.

## 2. Guest ABI (documented in `docs/site/guides/creating-a-validator.md`)

`wasm32-unknown-unknown`, no WASI, no imports except `env.log(ptr: i32, len: i32)` (debug
tracing at `debug` level, ≤ 1 KiB per call, ≤ 16 calls per validation). Exports:

```
memory                          // the guest's linear memory
alloc(len: i32) -> i32          // host asks for a buffer to write the input into
validate(ptr: i32, len: i32) -> i32   // 0 = accept; non-zero = refuse
reason_ptr() -> i32, reason_len() -> i32   // UTF-8 refusal reason after a non-zero return
```

Input (UTF-8 JSON, written by the host):

```jsonc
{ "docType": "actor", "op": "create" | "update" | "move",
  "system": { … post-image system band … },
  "prior":  { … pre-image system band … } | null,
  "name": "…" | null, "worldId": "…", "moduleId": "…" }
```

Nothing else: no `engine` band (engine-validated already, and engine-owned), no permissions, no
other documents, no clock, no randomness. Deterministic by construction.

## 3. Host (`src/server/src/sandbox/`)

- `sandbox::runtime::run_validator(module: &CompiledValidator, input: &ValidatorInput) ->
  ValidatorVerdict` on `tokio::task::spawn_blocking`: fresh `Store` per call with
  `StoreLimits { memory_size: 16 MiB, instances: 1, tables: 1 }`, `consume_fuel(true)` with
  `MAX_FUEL = 50_000_000`, instantiate, `alloc` → write input (input size ≤ 1 MiB, else refuse
  as `InputTooLarge` without running), call `validate`, read the reason (≤ 512 bytes,
  `String::from_utf8_lossy`, control characters stripped). Every trap, out-of-fuel, missing
  export, out-of-bounds pointer or non-UTF-8 reason ⇒ `Verdict::Fault(FaultKind)`. Wall clock
  is bounded by fuel; an additional `Instant` guard aborts reporting at 250 ms (belt and
  braces — measured per call and logged at `warn` when exceeded).
- **Placement.** Validators follow the tier-2 `validate_system_schema_tree` precedent
  exactly: they run in `apply_intent` ONLY — never in `apply_command` (the trusted
  undo/replay substrate) — plus ONE second call site, `SqliteRepository::import_world`'s own
  document loop (a GM-initiated bulk write that bypasses `apply_intent` structurally); both
  sites call the single helper `sandbox::validate_document(world, doc_type, system_post,
  system_prior, registry) -> ValidatorVerdict`. **They run OUTSIDE the write transaction**:
  the single-writer pool (`max_connections(1)`) serializes every `apply_intent` server-wide,
  so a validator held inside the transaction would throttle every hosted world. The sequence
  is: read the pre-image `system` band through the read-only pool (`open_read_only_pool`),
  run the validators on `spawn_blocking`, then open the write transaction — whose OCC
  pre-image check (`old`) refuses the write with `Conflict` if the document changed in
  between, so the validated post-image is the one that commits. For every touched document
  whose `system` band changed (or a Create), validators of modules enabled for the world with
  `validators_enabled` whose `doc_type` matches run in module-id order; the FIRST refusal
  rejects the whole intent with `DataError::OpFailed("validator <module-id>: <reason>")` —
  the existing variant `reject_reason` already maps to `RejectReason::Invalid`. **The reason
  text needs a wire channel that does not exist today:** `ServerMsg::Reject` gains `detail:
  Option<String>` (`#[serde(default)]`, ≤ 512 bytes, control characters stripped),
  `RejectSchema` mirrors it `.nullish()`, `WorldSession`'s `onReject(reason, detail)` passes
  it through, and `App.svelte`'s toast appends it to the `intent.rejected.invalid` text; the
  notification renders its message as a text node (verified by a test that a `<b>` in the
  detail arrives as literal characters). Embedded children are validated with their own
  `doc_type`. Server-origin writes that touch `system` (imports, world seeds) are validated
  the same way — the band, not the origin, decides.
- **Fault policy (D8) — decided in the data layer, acted on in the `ws` layer.** `data` can
  never reach `ws::room::Room` (the dependency is one-directional), and a side-effect write
  from inside the intent path on the 1-connection pool deadlocks. So: `sandbox::
  validate_document` returns `ValidatorVerdict::{Accept, Refuse{module, reason},
  Fault{module, kind}}`; `apply_intent`'s error carries the verdict (`DataError::OpFailed`
  with a structured `ValidatorFault` attached through a new `DataError::Validator(ValidatorFault)`
  variant that `reject_reason` maps to `Invalid` like `OpFailed`); the CALLER in `ws::conn`
  hands a `Fault` to `Room::note_validator_fault(module)`, which owns the in-memory
  per-(world, module) consecutive-fault counter (restart-resettable, beside `moving`/
  `session_floors`) and, at 5, issues the disable write (`set_world_enabled_modules` with the
  flag cleared — its own transaction, after the rejected intent is fully unwound) and posts
  the GM-only notice through the ordinary message Create path (`build_message_doc` +
  `Audience::GmOnly`, a second short transaction). Any accepted call resets the counter. A
  `Fault` refuses the write with the detail `"validator <module-id> faulted"`.
  **A slow-but-under-fuel validator is a fault too:** the per-call wall-clock guard (50 ms
  budget on the blocking thread; measured, not fuel-derived) converts an over-budget call
  into `Fault(TooSlow)` — so a validator that merely drags gets auto-disabled after 5 calls
  and cannot throttle the table indefinitely. Concurrency is bounded by construction: one
  intent at a time reaches the write pool, so at most one validator runs per world write and
  the blocking pool never holds more than the in-flight intents.
- **Scope of authority.** A validator can refuse; it cannot mutate (the host ignores the
  guest's memory after reading the reason), read other documents, observe time, or reach the
  network — there is no import for any of it. Fuel bounds CPU per write; memory limits bound
  RAM; module size bounds compile time; one instance per call bounds concurrency to the
  intent's own task.

## 4. Threat model (recorded in `docs/design/sandboxed-validators.md`, new)

| Threat | Mitigation |
|---|---|
| CPU exhaustion (infinite loop) | fuel cap per call; fault counter auto-disables after 5 |
| Slow-but-under-fuel validator throttling every world (the write pool is a single connection server-wide) | validators run OUTSIDE the write transaction against a read-only pre-image; a 50 ms wall-clock budget per call is a fault; 5 faults auto-disable |
| Many simultaneous intents each running a validator | bounded by the single-writer pool: one intent reaches the write path at a time per server, so at most one validator per in-flight intent; stated here so a future pool change re-examines it |
| Memory exhaustion | `StoreLimits` 16 MiB; module ≤ 4 MiB; input ≤ 1 MiB |
| Escaping the sandbox | `wasmi` is a pure-Rust interpreter with no JIT and no host imports beyond `log`; no WASI |
| Data exfiltration | no network/filesystem/clock imports; the only output channel is the ≤ 512-byte reason, visible to the writer who already holds the data |
| Denial of writes to the table | GM opt-in per world per module; the GM can turn it off; auto-disable on faults; the GM's own writes are subject to it too (a validator that refuses everything is visible on the first write) |
| Malicious reason text | carried on the NEW `Reject.detail` wire field, rendered as a text node only; length + control-char stripping |
| Supply chain (a validator swapped on disk) | out of scope here — Phase 4's signing/SRI row; the docs say so |

## 5. Example + docs

- `examples/validator-rust/`: a Cargo crate (`crate-type = ["cdylib"]`, target
  `wasm32-unknown-unknown`, `no_std` with a tiny bump allocator) that refuses an `actor`
  whose `system.hp` is negative. Three fixed rules: (1) a hand-rolled minimal JSON scan — no
  `serde-json-core` or any other dependency; (2) no build scripts — the docs guide documents
  the raw `cargo build --target wasm32-unknown-unknown --release` line; (3)
  `src/server/tests/sandbox.rs` ALWAYS builds the example itself through
  `std::process::Command` and fails loudly, naming `rustup target add wasm32-unknown-unknown`,
  when the target is missing — it never skips.
- CI: the `wasm32-unknown-unknown` target is added to the THREE-OS `rust` matrix job's
  `dtolnay/rust-toolchain` step (`targets:`), because `cargo test --all` runs there and
  `tests/sandbox.rs` builds the example on every leg; the `docs` job adds the same target so
  the guide's code-imported example builds. The contributing/hosting docs list the target as
  a toolchain prerequisite for `cargo test`.
- `docs/site/guides/creating-a-validator.md` (ABI, limits, the example, the opt-in UI);
  `creating-a-module.md` cross-links; the module portal page for `ModuleManager` documents the
  toggle; ARCHITECTURE §4's sandbox row and §5's Deno line rewritten (wasmi chosen, JIT
  runtimes rejected for size + attack surface); PLAN.md's "capability Phase 3" paragraph
  removed at merge.
- **`wasmi` is not in the tree yet, so every API name in §3 (`StoreLimits`, `consume_fuel`,
  fuel units) is unverified: the plan's first task adds the dependency, reads the resolved
  crate's docs, and corrects §3's names in this spec before any code is written.**

## 6. Tests

- `sandbox::runtime` unit tests with WAT-built modules (`wat` dev-dependency, MIT/Apache):
  accept (0), refuse with reason, refuse with an over-long reason (truncated), infinite loop ⇒
  `Fault(OutOfFuel)`, `memory.grow` bomb ⇒ `Fault(MemoryLimit)`, missing `validate` export ⇒
  `Fault(BadAbi)`, `alloc` returning out-of-bounds ⇒ `Fault(BadPointer)`, non-UTF-8 reason ⇒
  lossy, 16 `log` calls honoured and the 17th ignored, input over 1 MiB ⇒ refused without
  running, wall-clock guard logs.
- `modules.rs`: `validators` parse; traversal refused; bad wasm ⇒ `load_error` and the module
  still lists.
- `apply_intent`: a Create with valid `engine` and a validator-refused `system` is rejected
  with `Reject.detail` carrying the composed reason; `validators_enabled: false` ⇒ accepted;
  module disabled ⇒ accepted; embedded child validated by its own type; `import_world`
  refuses a bundle document a validator refuses; `apply_command` replay never runs a
  validator; two modules ⇒ module-id order and first refusal wins; a document changed between
  the pre-image read and the transaction ⇒ `Conflict`; the wall-clock guard yields
  `Fault(TooSlow)`. `Room::note_validator_fault`: 5 faults ⇒ flag cleared + GM notice, an
  accept resets. `WorldModuleEntry` round-trips; a legacy string-array setting reads as all
  flags false. Client: the toast shows `detail` as text (a `<b>` arrives literally).
- `tests/sandbox.rs` integration: the example validator built and installed into a temp
  `modules_dir`; GM enables module + validators; a player's `hp: -1` actor create is rejected
  with the reason; `hp: 3` accepted.
- Client: `ModuleManager` toggle (GM-only, only when validators declared, load error shown);
  manifest schema accepts the key.
- Binary size: `pnpm lint:binary-size` measured before and after (recorded in the HISTORY
  entry).

## 7. Skills

New `shadowcat-codebase-sandbox` (master §6); updates to `module-toolchain` (manifest key,
registry) and `documents-permissions` (the validation placement). Hook globs
`src/server/src/sandbox/`, `examples/validator-rust/`.
