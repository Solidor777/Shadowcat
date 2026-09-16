//! Pure-helper tests for `audio_monitor`'s wire-safety functions: basename reduction,
//! truncation, clamping, watch-list matching, and the combined filter pipeline. The
//! per-backend enumeration smoke test lives beside each backend (`windows/tests.rs`,
//! `macos/tests.rs`, `linux/tests.rs`) so it only compiles (and runs) on its own OS.

use super::*;

#[test]
fn reduce_to_basename_strips_the_directory() {
    assert_eq!(
        reduce_to_basename("C:\\Program Files\\Discord\\Discord.exe"),
        "Discord.exe"
    );
    assert_eq!(reduce_to_basename("/usr/bin/discord"), "discord");
    assert_eq!(reduce_to_basename("discord"), "discord");
}

#[test]
fn truncate_process_name_caps_at_128_unicode_scalars_not_bytes() {
    let name: String = "é".repeat(200);
    let truncated = truncate_process_name(&name);
    assert_eq!(truncated.chars().count(), PROCESS_NAME_MAX_CHARS);
}

#[test]
fn clamp_peak_bounds_to_unit_range() {
    assert_eq!(clamp_peak(-0.5), 0.0);
    assert_eq!(clamp_peak(1.5), 1.0);
    assert_eq!(clamp_peak(0.42), 0.42);
}

#[test]
fn matches_watch_list_is_case_insensitive_substring() {
    let watch = vec!["discord".to_string()];
    assert!(matches_watch_list("Discord.exe", &watch));
    assert!(matches_watch_list("DISCORD", &watch));
    assert!(!matches_watch_list("firefox", &watch));
}

#[test]
fn matches_watch_list_empty_watch_matches_nothing() {
    assert!(!matches_watch_list("discord", &[]));
}

#[test]
fn filter_for_watch_list_reduces_clamps_truncates_and_filters() {
    let raw = vec![
        SessionLevel {
            process: "/usr/bin/discord".to_string(),
            peak: 1.5,
        },
        SessionLevel {
            process: "/usr/bin/firefox".to_string(),
            peak: 0.3,
        },
    ];
    let filtered = filter_for_watch_list(raw, &["discord".to_string()]);
    assert_eq!(
        filtered,
        vec![SessionLevel {
            process: "discord".to_string(),
            peak: 1.0
        }]
    );
}

#[test]
fn fake_monitor_replays_script_then_repeats_last() {
    use fake::FakeMonitor;
    let mut m = FakeMonitor::new(vec![
        Ok(vec![SessionLevel {
            process: "discord".to_string(),
            peak: 0.1,
        }]),
        Err(MonitorError::Backend("transient".to_string())),
    ]);
    assert_eq!(m.poll().unwrap()[0].peak, 0.1);
    assert!(m.poll().is_err());
    assert!(m.poll().is_err()); // repeats the last scripted entry
}
