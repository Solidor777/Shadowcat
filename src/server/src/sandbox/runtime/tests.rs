use super::*;
use crate::sandbox::{ValidatorFault, ValidatorInput};

/// Wall-clock budgets that keep the production `HANG_GUARD` out of every non-timing test:
/// on a saturated CI runner, instantiating even a trivial module can exceed the production
/// guard's wall clock, which would race the verdict kind the test actually pins (a
/// non-timing assertion must never observe `Fault(TooSlow)`). Tests that pin the
/// guard/slow-call behavior itself set their own budgets explicitly instead.
const GENEROUS: (Duration, Duration) = (Duration::from_secs(3600), Duration::from_secs(3600));

/// Compiles a WAT fixture into a `CompiledValidator` through `CompiledValidator::compile`,
/// the same single compile path the registry's scan-time compile uses.
fn compiled(wat_src: &str) -> CompiledValidator {
    let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
    CompiledValidator::compile("test-module", &bytes).expect("module compiles")
}

fn input(hp: i64) -> ValidatorInput {
    ValidatorInput {
        doc_type: "item".into(),
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
    let verdict =
        run_validator_with_budgets(&compiled(ACCEPT_WAT), &input(10), GENEROUS.0, GENEROUS.1).await;
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
    let verdict =
        run_validator_with_budgets(&compiled(REFUSE_WAT), &input(-1), GENEROUS.0, GENEROUS.1).await;
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
    let verdict = run_validator_with_budgets(
        &compiled(REFUSE_LONG_REASON_WAT),
        &input(-1),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
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
    // Generous wall-clock budgets: how long burning `MAX_FUEL` takes is machine- and
    // build-profile-dependent, and where it exceeds the production `HANG_GUARD` the guard
    // legitimately fires first (both kinds count toward auto-disable identically — the
    // guard-first race is exactly what `a_call_that_never_returns_faults_hung...` pins).
    // This test isolates the fuel cap itself, so it must not race the guard.
    let verdict = run_validator_with_budgets(
        &compiled(INFINITE_LOOP_WAT),
        &input(0),
        Duration::from_secs(3600),
        Duration::from_secs(3600),
    )
    .await;
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
/// out-of-bounds access is what traps. Bounded: the ceiling (`MAX_MEMORY_BYTES`, 256 pages)
/// is reached in roughly 256 grow calls, far below the fuel budget.
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
    let verdict = run_validator_with_budgets(
        &compiled(MEMORY_BOMB_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
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
    let verdict = run_validator_with_budgets(
        &compiled(MISSING_VALIDATE_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
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
    let verdict = run_validator_with_budgets(
        &compiled(BAD_POINTER_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
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
    let verdict = run_validator_with_budgets(
        &compiled(NON_UTF8_REASON_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
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
    let verdict = run_validator_with_budgets(
        &compiled(LOG_SEVENTEEN_TIMES_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
    assert_eq!(verdict, ValidatorVerdict::Accept);
}

#[tokio::test]
async fn input_over_one_mib_is_refused_without_running() {
    let mut oversized = input(0);
    oversized.name = Some("x".repeat(2 * 1024 * 1024));
    let verdict =
        run_validator_with_budgets(&compiled(ACCEPT_WAT), &oversized, GENEROUS.0, GENEROUS.1).await;
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

#[tokio::test]
async fn a_trapping_call_keeps_its_precise_kind_even_past_the_slow_call_budget() {
    // A zero-duration budget is exceeded by any measurable call, but a call that TRAPPED is
    // not reclassified: an infinite loop must surface as `OutOfFuel`, never `TooSlow` — the
    // trap's own kind is strictly more diagnostic, and both count toward auto-disable
    // identically.
    let verdict = run_validator_with_budgets(
        &compiled(INFINITE_LOOP_WAT),
        &input(0),
        Duration::ZERO,
        Duration::from_secs(3600),
    )
    .await;
    assert_eq!(
        verdict,
        ValidatorVerdict::Fault(ValidatorFault {
            module: "test-module".into(),
            kind: FaultKind::OutOfFuel,
            consecutive: 0,
        })
    );
}

/// Calls `env.log` with negative and overflowing (ptr, len) pairs; the host drops them
/// instead of wrapping into a wild read, and the verdict is unaffected.
const MALFORMED_LOG_ARGS_WAT: &str = r#"
  (module
    (import "env" "log" (func $log (param i32 i32)))
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0))
    (func (export "validate") (param i32 i32) (result i32)
      (call $log (i32.const -1) (i32.const -1))
      (call $log (i32.const 0) (i32.const -1))
      (call $log (i32.const 2147483647) (i32.const 2147483647))
      (i32.const 0)))
"#;

#[tokio::test]
async fn malformed_log_args_are_dropped_not_a_fault() {
    let verdict = run_validator_with_budgets(
        &compiled(MALFORMED_LOG_ARGS_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
    assert_eq!(verdict, ValidatorVerdict::Accept);
}

/// `reason_ptr` traps (unreachable) instead of returning — a trap, never a pointer-shape
/// problem.
const TRAPPING_REASON_PTR_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
    (func (export "reason_ptr") (result i32) unreachable)
    (func (export "reason_len") (result i32) (i32.const 2)))
"#;

#[tokio::test]
async fn a_trapping_reason_export_is_classified_as_a_trap_not_a_bad_pointer() {
    let verdict = run_validator_with_budgets(
        &compiled(TRAPPING_REASON_PTR_WAT),
        &input(0),
        GENEROUS.0,
        GENEROUS.1,
    )
    .await;
    let ValidatorVerdict::Fault(ValidatorFault { kind, .. }) = verdict else {
        panic!("expected Fault, got {verdict:?}");
    };
    assert_eq!(kind, FaultKind::Trap);
}
