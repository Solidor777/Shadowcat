//! `accept_error_pause` logs exactly the errors that pause. Runs on the default current-thread
//! runtime so the log capture sees the lines.

use std::io;

use shadowcat::http::conn_serve::{accept_error_pause, ACCEPT_ERROR_PAUSE};
use shadowcat_test_support::logs::capture_current_thread;

#[test]
fn a_pausing_error_is_logged_once_and_yields_the_pause() {
    let (_guard, log) = capture_current_thread();
    let pause = accept_error_pause(&io::Error::other("out of descriptors"));
    assert_eq!(pause, Some(ACCEPT_ERROR_PAUSE));
    let text = log.text();
    assert_eq!(text.matches("accept error").count(), 1, "{text}");
    assert!(text.contains("out of descriptors"), "{text}");
}

#[test]
fn a_connection_level_error_is_neither_logged_nor_paused() {
    let (_guard, log) = capture_current_thread();
    let pause = accept_error_pause(&io::Error::from(io::ErrorKind::ConnectionReset));
    assert_eq!(pause, None);
    assert!(!log.text().contains("accept error"), "{}", log.text());
}
