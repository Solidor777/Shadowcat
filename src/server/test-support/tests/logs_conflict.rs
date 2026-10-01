//! A test binary that already holds its own global subscriber must not get a silent, empty
//! capture sink.

#[test]
#[should_panic(expected = "a different global tracing subscriber")]
fn capture_panics_when_another_global_subscriber_is_installed() {
    tracing_subscriber::fmt().with_writer(std::io::sink).init();
    let _ = shadowcat_test_support::logs::capture_current_thread();
}
