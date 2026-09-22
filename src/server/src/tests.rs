//! Tests for `build_rolling_appender`'s log-directory bootstrap.

use super::*;

/// A fresh, guaranteed-nonexistent child of a freshly created `TempDir` this test owns
/// exclusively. The returned `TempDir` must be kept alive for the test's duration -- dropping it
/// recursively removes both it and the not-yet-existing child once the test is done, so no test
/// deletes a directory itself.
fn fresh_temp_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = tempfile::tempdir().expect("create temp dir");
    let dir = base.path().join("logs");
    (base, dir)
}

/// `build_rolling_appender` must create `dir` itself before the vendored
/// `tracing-appender` internals ever run their prune-then-write cycle over it -- a fresh
/// install's log directory does not exist on first start, and the vendored crate's own
/// `Inner::new` prunes (a `read_dir`) BEFORE it creates the directory. Verified two ways:
/// (1) the directory exists afterward, and a rolling log file exists inside it once at least
/// one event has been written and flushed; (2) the precondition the vendored crate's directory-
/// read error depends on -- a missing directory at prune time -- cannot occur here, since this
/// function creates `dir` before `Builder::build` (which triggers that prune) ever runs. Point
/// (2) is a structural argument, not a captured-stderr assertion: the vendored crate's error
/// path writes via a bare `eprintln!` with no hook this crate can intercept without redirecting
/// the OS-level file descriptor, so this test verifies the precondition the error depends on
/// instead of the absence of the error's own output.
#[test]
fn build_rolling_appender_creates_a_fresh_log_directory_before_writing() {
    use std::io::Write;

    let (_base, dir) = fresh_temp_dir();
    assert!(
        !dir.exists(),
        "test setup: this must start from a genuinely nonexistent directory"
    );

    let mut appender = build_rolling_appender(&dir)
        .expect("must create the missing directory itself rather than fail on it");
    appender
        .write_all(b"probe event\n")
        .expect("write one event to the fresh appender");
    appender.flush().expect("flush the probe event to disk");
    drop(appender);

    assert!(
        dir.is_dir(),
        "build_rolling_appender must create the log directory before returning"
    );
    let has_log_file = std::fs::read_dir(&dir)
        .expect("directory was just asserted to exist above")
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("shadowcat.") && name.ends_with(".log")
        });
    assert!(
        has_log_file,
        "a rolling log file must exist once the appender has flushed at least one event"
    );
}

/// A directory that already exists (the common case on every start after the first) is left
/// alone by `create_dir_all` and the appender still writes into it normally -- the fix must not
/// regress the steady-state path.
#[test]
fn build_rolling_appender_tolerates_an_already_existing_directory() {
    use std::io::Write;

    let (_base, dir) = fresh_temp_dir();
    std::fs::create_dir_all(&dir).expect("test setup: pre-create the directory");

    let mut appender =
        build_rolling_appender(&dir).expect("an already-existing directory must not be an error");
    appender.write_all(b"probe event\n").expect("write");
    appender.flush().expect("flush");
    drop(appender);

    assert!(dir.is_dir());
}
