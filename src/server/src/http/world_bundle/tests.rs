use super::*;

/// Exercises the shared cap check with a small `cap`, rather than
/// against `MAX_IMPORT_BUNDLE_BYTES` (2 GiB), which no test should
/// actually materialize.
#[test]
fn check_upload_cap_rejects_only_once_the_total_exceeds_cap() {
    assert!(check_upload_cap(0, 100).is_ok());
    assert!(check_upload_cap(100, 100).is_ok(), "exactly at cap is fine");
    let err = check_upload_cap(101, 100).unwrap_err();
    assert!(matches!(err, AppError::PayloadTooLarge(_)));
}

/// Confirms `ChannelWriter` genuinely backpressures its blocking writer
/// against the channel's capacity, rather than buffering ahead of it —
/// the property `export_world`'s memory-bounded streaming claim rests
/// on. With capacity 1, a writer producing two chunks must block inside
/// the second `blocking_send` until the first chunk is drained: with
/// nothing draining the channel, the writer thread cannot have finished
/// both writes no matter how long it runs — proven via a completion
/// flag the writer thread sets only after `write_all` for "second"
/// returns, never via a `try_recv` race with the writer thread itself.
#[test]
fn channel_writer_blocks_the_writer_thread_once_the_channel_is_full() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let mut writer = ChannelWriter { tx };
    let second_write_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_flag = second_write_done.clone();

    let handle = std::thread::spawn(move || {
        std::io::Write::write_all(&mut writer, b"first").unwrap();
        std::io::Write::write_all(&mut writer, b"second").unwrap();
        done_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Give the writer thread ample time to run — if it were not
    // backpressured by the full channel, both writes (and the flag
    // store) would already have completed.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !second_write_done.load(std::sync::atomic::Ordering::SeqCst),
        "second write completed with the channel undrained — no backpressure"
    );

    // Draining the one slot is what unblocks the writer thread.
    let first = rx.blocking_recv().expect("first chunk").unwrap();
    assert_eq!(first, b"first");
    let second = rx.blocking_recv().expect("second chunk").unwrap();
    assert_eq!(second, b"second");

    handle.join().unwrap();
    assert!(second_write_done.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn temp_file_guard_removes_the_file_on_ordinary_drop() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("guarded.tmp");
    std::fs::write(&path, b"x").unwrap();
    {
        let _guard = TempFileGuard::new(path.clone());
    }
    assert!(!path.exists());
}

#[test]
fn temp_file_guard_keep_prevents_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("kept.tmp");
    std::fs::write(&path, b"x").unwrap();
    let guard = TempFileGuard::new(path.clone());
    guard.keep();
    assert!(path.exists());
}

/// Models the exact shape `import_world` relies on: a guard held across
/// a fallible operation, then an early return via `?` (the shape a
/// `spawn_blocking(read_bundle)` panic takes once `JoinError` propagates
/// out of the `map_err(..)?` — a normal early return, not a Rust panic,
/// since `spawn_blocking` converts a panicked task into an `Err` value).
/// The guard local goes out of scope on that early return exactly as it
/// would on any other path, so cleanup is unconditional.
#[test]
fn temp_file_guard_removes_the_file_on_early_return_via_question_mark() {
    fn helper(path: std::path::PathBuf) -> Result<(), AppError> {
        let _guard = TempFileGuard::new(path);
        let inner: Result<(), AppError> = Err(AppError::Internal);
        inner?;
        Ok(())
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("early-return.tmp");
    std::fs::write(&path, b"x").unwrap();
    assert!(helper(path.clone()).is_err());
    assert!(!path.exists());
}

/// A genuine Rust panic while the guard is live — belt-and-suspenders on
/// top of the `?`-based test above, confirming `Drop::drop` also runs
/// during unwind, not just on a normal early return.
#[test]
fn temp_file_guard_removes_the_file_on_panic_unwind() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("panicked.tmp");
    std::fs::write(&path, b"x").unwrap();
    let path_for_panic = path.clone();
    let result = std::panic::catch_unwind(move || {
        let _guard = TempFileGuard::new(path_for_panic);
        panic!("simulated panic while the guard is live");
    });
    assert!(result.is_err());
    assert!(
        !path.exists(),
        "guard must remove the file during panic unwind"
    );
}
