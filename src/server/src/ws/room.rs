//! Per-world rooms, ring buffer, registry, and telemetry counters.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::ws::protocol::ServerMsg;

const MAX_EVENTS: usize = 1024;
const MAX_AGE_MS: i64 = 5 * 60 * 1000;

/// Recent `Event` frames for hot resync, bounded by count and age. Age is
/// measured relative to the newest buffered event's `ts`.
pub struct RingBuffer {
    events: VecDeque<Arc<ServerMsg>>, // ascending seq; each is ServerMsg::Event
}

impl RingBuffer {
    pub fn new() -> Self {
        Self { events: VecDeque::new() }
    }

    /// Append an `Event` frame and prune by count then age.
    pub fn push(&mut self, msg: Arc<ServerMsg>) {
        debug_assert!(msg.event_seq().is_some(), "only Event frames are buffered");
        self.events.push_back(msg);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        if let Some(newest) = self.events.back().and_then(|m| m.event_ts()) {
            while let Some(oldest) = self.events.front().and_then(|m| m.event_ts()) {
                if newest - oldest > MAX_AGE_MS {
                    self.events.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    /// Events with `seq >= from_seq`, but only when the whole requested range is
    /// still resident (oldest buffered seq <= from_seq). Otherwise `None` so the
    /// caller falls back to the durable `events_since` cold tier. An empty buffer
    /// returns `None` (cannot prove residency).
    pub fn range_from(&self, from_seq: i64) -> Option<Vec<Arc<ServerMsg>>> {
        match self.events.front().and_then(|m| m.event_seq()) {
            Some(oldest) if oldest <= from_seq => Some(
                self.events
                    .iter()
                    .filter(|m| m.event_seq().map(|s| s >= from_seq).unwrap_or(false))
                    .cloned()
                    .collect(),
            ),
            _ => None,
        }
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;
    use crate::data::command::Command;
    use uuid::Uuid;

    fn event(seq: i64, ts: i64) -> Arc<ServerMsg> {
        Arc::new(ServerMsg::Event {
            command: Command {
                seq,
                world_id: Uuid::from_u128(1),
                author: Uuid::from_u128(2),
                ts,
                ops: vec![],
            },
        })
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
        assert!(rb.range_from(1).is_none(), "seq 1 evicted, range not fully resident");
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
}
