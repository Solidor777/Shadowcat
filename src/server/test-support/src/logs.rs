//! Log capture for tests that assert on what the server logs.
//!
//! Capturing the tracing output a piece of code emits, for tests that assert on log content.

use std::cell::RefCell;
use std::io::Write;
use std::sync::{Arc, Mutex, Once};

/// A log sink a test can read back.
#[derive(Clone, Default)]
pub struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl LogBuf {
    /// Everything written so far, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

thread_local! {
    /// The sink the current thread's log lines go to, if a capture is active on it.
    static SINK: RefCell<Option<LogBuf>> = const { RefCell::new(None) };
}

/// Routes every line the process-wide subscriber formats to the current thread's active sink.
#[derive(Clone, Copy)]
struct ThreadRouter;

impl Write for ThreadRouter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        SINK.with(|sink| {
            if let Some(target) = sink.borrow_mut().as_mut() {
                target.write_all(buf)?;
            }
            Ok(buf.len())
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ThreadRouter {
    type Writer = ThreadRouter;

    fn make_writer(&'a self) -> ThreadRouter {
        *self
    }
}

/// Installs the process-wide INFO-level, colour-free subscriber once. A test binary that already
/// holds a global subscriber keeps it; [`capture_current_thread`] detects that with a sentinel
/// line and panics rather than returning a sink that stays empty.
///
/// Algorithm: a single global subscriber whose writer is a per-thread sink lookup. Source: the
/// `tracing` callsite-interest cache (`tracing-core` `callsite`): a callsite first hit on a thread
/// with no scoped dispatcher can be cached as disabled, so a scoped subscriber installed by a
/// concurrent test intermittently sees nothing. A global subscriber keeps every callsite enabled
/// at INFO, and capture is selected per thread by the sink instead.
fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // An `Err` means another global subscriber exists; the sentinel check in
        // `capture_current_thread` turns that into a loud failure.
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(ThreadRouter)
                .with_max_level(tracing::Level::INFO)
                .with_ansi(false)
                .finish(),
        );
    });
}

/// Ends a capture on the current thread when dropped. Not `Send`: the capture is tied to the
/// thread that started it, so dropping the guard on another thread would clear the wrong sink.
///
/// # Examples
///
/// The type resolves and is usable (the twin of the `compile_fail` example below, which would
/// otherwise also pass if the path were misspelled):
///
/// ```
/// fn assert_type<T>() {}
/// assert_type::<shadowcat_test_support::logs::CaptureGuard>();
/// ```
///
/// The same type is not `Send`:
///
/// ```compile_fail,E0277
/// fn assert_send<T: Send>() {}
/// assert_send::<shadowcat_test_support::logs::CaptureGuard>();
/// ```
pub struct CaptureGuard(std::marker::PhantomData<*const ()>);

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        SINK.with(|sink| *sink.borrow_mut() = None);
    }
}

/// Runs the synchronous `f` with the current thread's log lines captured and returns them.
///
/// # Examples
///
/// ```
/// let log = shadowcat_test_support::logs::capture_info(|| tracing::info!("hello"));
/// assert!(log.contains("hello"));
/// ```
pub fn capture_info(f: impl FnOnce()) -> String {
    let (guard, buf) = capture_current_thread();
    f();
    drop(guard);
    buf.text()
}

/// Captures the CURRENT THREAD's INFO-level log lines into the returned sink until the guard
/// drops. Only lines emitted on this thread are captured, so use it with synchronous code or an
/// async test on a current-thread runtime (`#[tokio::test]` default), where the code under test
/// logs from the test's own task; lines emitted on another thread, such as a multi-thread
/// runtime's workers, are not captured.
///
/// # Panics
///
/// Panics when the process already holds a different global `tracing` subscriber, because the
/// sink would otherwise stay empty and every absence assertion would pass vacuously.
///
/// # Examples
///
/// ```
/// let (guard, buf) = shadowcat_test_support::logs::capture_current_thread();
/// tracing::info!("hello");
/// drop(guard);
/// assert!(buf.text().contains("hello"));
/// ```
pub fn capture_current_thread() -> (CaptureGuard, LogBuf) {
    install();
    // Sentinel round trip: prove this module's subscriber is the one receiving events.
    let probe = LogBuf::default();
    SINK.with(|sink| *sink.borrow_mut() = Some(probe.clone()));
    tracing::info!("log capture sentinel");
    assert!(
        probe.text().contains("log capture sentinel"),
        "log capture is unavailable: a different global tracing subscriber is installed in this test binary"
    );
    let buf = LogBuf::default();
    SINK.with(|sink| *sink.borrow_mut() = Some(buf.clone()));
    (CaptureGuard(std::marker::PhantomData), buf)
}
