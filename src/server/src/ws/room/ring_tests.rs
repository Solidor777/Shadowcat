use super::*;
use crate::data::command::Command;
use uuid::Uuid;

fn event(seq: i64, ts: i64) -> RoomEvent {
    RoomEvent::Event(Arc::new(StoredCommand {
        command: Command {
            seq,
            world_id: Uuid::from_u128(1),
            author: Uuid::from_u128(2),
            ts,
            ops: vec![],
        },
        snapshot: crate::data::snapshot::CommandSnapshot {
            per_op: vec![],
            world_gm_at_commit: std::collections::HashMap::new(),
        },
    }))
}

#[test]
fn evicts_by_count() {
    let mut rb = RingBuffer::new();
    for s in 1..=(MAX_EVENTS as i64 + 10) {
        rb.push(event(s, 0));
    }
    // Only the newest MAX_EVENTS are retained; oldest resident is seq 11.
    let all = rb.range_from(11).unwrap();
    assert_eq!(all.len(), MAX_EVENTS);
    assert_eq!(all.first().unwrap().event_seq().unwrap(), 11);
    // Seq 1..=10 evicted: a from_seq below the resident floor is not serviceable.
    assert!(rb.range_from(1).is_none());
}

#[test]
fn evicts_by_age_relative_to_newest() {
    let mut rb = RingBuffer::new();
    rb.push(event(1, 0));
    rb.push(event(2, 100));
    rb.push(event(3, MAX_AGE_MS + 1)); // pushes seq 1 (age > MAX) out
    assert!(
        rb.range_from(1).is_none(),
        "seq 1 evicted, range not fully resident"
    );
    let r = rb.range_from(2).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].event_seq().unwrap(), 2);
}

#[test]
fn range_from_returns_suffix_when_resident() {
    let mut rb = RingBuffer::new();
    for s in 1..=5 {
        rb.push(event(s, 0));
    }
    let r = rb.range_from(3).unwrap();
    assert_eq!(
        r.iter().map(|m| m.event_seq().unwrap()).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );
}

#[test]
fn range_from_none_when_requested_seq_evicted() {
    let mut rb = RingBuffer::new();
    for s in 1..=(MAX_EVENTS as i64 + 5) {
        rb.push(event(s, 0));
    }
    // oldest resident is 6; asking from 1 cannot be fully served from buffer.
    assert!(rb.range_from(1).is_none());
}

#[test]
fn range_from_none_on_empty_buffer() {
    let rb = RingBuffer::new();
    assert!(rb.range_from(1).is_none());
}
