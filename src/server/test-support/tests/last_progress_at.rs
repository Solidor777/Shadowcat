//! `WriteStats::last_progress_at` names the grant at which the bytes accepted since reach the
//! floor, so a late small write after a lump does not move the reference, and a late lump does.

use std::time::Duration;

use shadowcat_test_support::observed::WriteStats;
use tokio::time::Instant;

const FLOOR: u64 = 16 * 1024;

#[test]
fn a_late_lump_is_the_last_progress() {
    let stats = WriteStats::default();
    let t0 = Instant::now();
    stats.note_accepted(t0, 100_000);
    stats.note_accepted(t0 + Duration::from_millis(30), 150_000);
    let late = t0 + Duration::from_secs(3);
    stats.note_accepted(late, 120_000);
    assert_eq!(stats.last_progress_at(FLOOR), Some(late));
}

#[test]
fn writes_that_sum_under_the_floor_after_a_lump_do_not_move_the_reference() {
    let stats = WriteStats::default();
    let t0 = Instant::now();
    stats.note_accepted(t0, 100_000);
    // Acknowledgement-sized writes: together far under the floor.
    stats.note_accepted(t0 + Duration::from_secs(1), 17);
    stats.note_accepted(t0 + Duration::from_secs(2), 17);
    assert_eq!(stats.last_progress_at(FLOOR), Some(t0));
}

#[test]
fn small_writes_that_reach_the_floor_together_count_at_the_grant_that_completes_it() {
    let stats = WriteStats::default();
    let t0 = Instant::now();
    stats.note_accepted(t0, 8 * 1024);
    stats.note_accepted(t0 + Duration::from_secs(1), 8 * 1024);
    stats.note_accepted(t0 + Duration::from_secs(2), 8 * 1024);
    // Walking back: 8 KiB (2 s), 16 KiB (1 s) reaches the floor at the 1 s grant.
    assert_eq!(
        stats.last_progress_at(FLOOR),
        Some(t0 + Duration::from_secs(1))
    );
}

#[test]
fn a_socket_that_never_accepted_the_floor_has_no_progress() {
    let stats = WriteStats::default();
    stats.note_accepted(Instant::now(), FLOOR - 1);
    assert_eq!(stats.last_progress_at(FLOOR), None);
}
