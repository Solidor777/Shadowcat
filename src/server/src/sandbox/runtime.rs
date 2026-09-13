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

/// One compiled, cached validator: the fuel-metered `Engine` plus the wasmi `Module` and the
/// declaring module/doc_type this validator belongs to (for fault reporting).
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::runtime::CompiledValidator;
///
/// let wasm = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
/// let compiled = CompiledValidator::compile("example-module", &wasm).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct CompiledValidator {
    /// The engine `module` was compiled with — a `wasmi::Module` is bound to its compiling
    /// engine, so a per-call fresh engine can never instantiate it ("foreign entity" panic);
    /// the engine is `Arc`-cheap to clone and `Send`/`Sync`.
    pub(super) engine: Engine,
    /// The compiled wasmi module.
    pub(super) module: Module,
    /// The declaring installed-module id (for `ValidatorVerdict`'s `module` field).
    pub(super) module_id: String,
}

impl CompiledValidator {
    /// Compiles `wasm_bytes` into a validator for `module_id` on a fresh fuel-metered engine —
    /// the ONE compile path, shared by `super::registry`'s scan-time compile (so an on-disk
    /// `.wasm` and a test fixture take the identical route). Compilation only parses and
    /// validates the module; no guest code runs until `run_validator` instantiates it.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::sandbox::runtime::CompiledValidator;
    ///
    /// let wasm = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
    /// assert!(CompiledValidator::compile("example-module", &wasm).is_ok());
    /// ```
    pub fn compile(module_id: &str, wasm_bytes: &[u8]) -> Result<Self, wasmi::Error> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_bytes)?;
        Ok(Self {
            engine,
            module,
            module_id: module_id.to_string(),
        })
    }
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
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() {
/// use shadowcat::sandbox::runtime::{run_validator, CompiledValidator};
/// use shadowcat::sandbox::{ValidatorInput, ValidatorVerdict};
///
/// // A validator whose `validate` always returns 0 accepts every write.
/// let wasm = wat::parse_str(r#"
///   (module
///     (memory (export "memory") 1)
///     (func (export "alloc") (param i32) (result i32) (i32.const 0))
///     (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
/// "#)
/// .unwrap();
/// let compiled = CompiledValidator::compile("example-module", &wasm).unwrap();
/// let input = ValidatorInput {
///     doc_type: "item".into(),
///     op: "create".into(),
///     system: serde_json::json!({}),
///     prior: None,
///     name: None,
///     world_id: uuid::Uuid::nil(),
///     module_id: "example-module".into(),
/// };
/// assert_eq!(run_validator(&compiled, &input).await, ValidatorVerdict::Accept);
/// # }
/// ```
pub async fn run_validator(
    compiled: &CompiledValidator,
    input: &ValidatorInput,
) -> ValidatorVerdict {
    run_validator_with_budgets(compiled, input, SLOW_CALL_BUDGET, HANG_GUARD).await
}

/// Runs `compiled` against `input` on `tokio::task::spawn_blocking`, on a fresh `Store` per
/// call. Every trap, out-of-fuel, missing export, out-of-bounds pointer or non-UTF-8 reason
/// (lossy-decoded, never a fault) maps to a `ValidatorVerdict::Fault`; a `validate` return of
/// `0` is `Accept`; any other return reads the reason via `reason_ptr`/`reason_len`. Every
/// `Fault` this function produces carries `consecutive: 0` — this function has no world or
/// registry context to compute the real streak; `sandbox::validate_document` stamps the real
/// value in before returning the verdict to ITS OWN caller. The slow-call budget reclassifies
/// only a call that COMPLETED (an authored `Accept`/`Refuse`) past budget as
/// `Fault(FaultKind::TooSlow)` — a call that trapped already carries its precise `FaultKind`,
/// which reclassification would only obscure (an infinite loop must surface as `OutOfFuel`,
/// not `TooSlow`); both kinds count toward auto-disable identically.
/// `slow_call_budget`/`hang_guard` are parameters (not the module consts directly) so
/// `runtime::tests` can shrink `hang_guard` far below the wall-clock cost of exhausting
/// `MAX_FUEL`, proving the guard's own `tokio::time::timeout` fires for real rather than
/// merely observing a fault fuel exhaustion would have produced anyway.
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
    let join = tokio::task::spawn_blocking(move || run_validator_blocking(&compiled, &bytes));
    let (result, elapsed) = match tokio::time::timeout(hang_guard, join).await {
        Ok(Ok(pair)) => pair,
        Ok(Err(_)) => return fault(FaultKind::Trap),
        Err(_) => {
            tracing::warn!(module = %module_id, "validator call exceeded the hang guard");
            return fault(FaultKind::Hung);
        }
    };
    if elapsed > slow_call_budget && !matches!(result, ValidatorVerdict::Fault(_)) {
        tracing::warn!(module = %module_id, ?elapsed, "validator call exceeded the slow-call budget");
        return fault(FaultKind::TooSlow);
    }
    result
}

/// The synchronous wrapper run inside `spawn_blocking`: times `run_validator_body` ON THIS
/// THREAD and pairs its verdict with the measured duration — the slow-call budget is defined
/// over the call's `Instant`-measured duration on the blocking thread, which deliberately
/// excludes `spawn_blocking` scheduling latency.
fn run_validator_blocking(
    compiled: &CompiledValidator,
    input_bytes: &[u8],
) -> (ValidatorVerdict, Duration) {
    let started = Instant::now();
    let verdict = run_validator_body(compiled, input_bytes);
    (verdict, started.elapsed())
}

/// The synchronous body: fresh `Store`/`Linker` on the engine that compiled
/// `compiled.module` (a `wasmi::Module` is bound to its compiling engine — a per-call fresh
/// engine can never instantiate it), instantiate, `alloc` → write input → `validate` → read
/// reason.
fn run_validator_body(compiled: &CompiledValidator, input_bytes: &[u8]) -> ValidatorVerdict {
    let module_id = compiled.module_id.clone();
    let fault = |kind: FaultKind| {
        ValidatorVerdict::Fault(ValidatorFault {
            module: module_id.clone(),
            kind,
            consecutive: 0,
        })
    };

    let engine = compiled.engine.clone();

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
                let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
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

    // `Linker::instantiate` is deprecated in favour of the fused `instantiate_and_start`; a
    // failure carrying a typed trap code came from the module's `start` function (a trap),
    // anything else is a missing/mistyped import — the module's own ABI problem.
    let instance = match linker.instantiate_and_start(&mut store, &compiled.module) {
        Ok(i) => i,
        Err(e) if e.as_trap_code().is_some() => return fault(classify_trap(&e)),
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

    match read_reason(&instance, &mut store, &memory) {
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
) -> Result<String, FaultKind> {
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
    Ok(text)
}

/// wasmi trap classification through the TYPED trap code, never the error's `Display` text:
/// `wasmi::Error::as_trap_code` yields the `TrapCode` a trap carries (`OutOfFuel` when the
/// store's fuel is exhausted, `MemoryOutOfBounds`/`TableOutOfBounds` when the guest addresses
/// past its current memory). A `memory.grow` the `StoreLimits` ceiling denies never traps —
/// core Wasm returns -1 to the guest — so the ceiling itself is invisible here; it becomes a
/// `MemoryLimit` fault only when the guest then touches memory it failed to obtain. Any other
/// trap code, or an error carrying no trap code (a host-import error), is a generic `Trap`.
/// `TrapCode` is read from the crate root — the `wasmi::core` re-export module is deprecated.
fn classify_trap(e: &wasmi::Error) -> FaultKind {
    use wasmi::TrapCode;
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => FaultKind::OutOfFuel,
        Some(TrapCode::MemoryOutOfBounds | TrapCode::TableOutOfBounds) => FaultKind::MemoryLimit,
        _ => FaultKind::Trap,
    }
}

#[cfg(test)]
mod tests;
