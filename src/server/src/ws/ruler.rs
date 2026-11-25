//! Shared live ruler: the ingress bounds and normalization for `ClientMsg::RulerUpdate`.
//!
//! A ruler is a measuring user's route preview relayed to the room. Its points are
//! client-supplied scene coordinates relayed as-is, the same unclipped posture `ScenePing`
//! takes for its coordinates: every connection in the room, the sender's own included, receives
//! the route as sent (the sender thins it to at most `MAX_RULER_POINTS`), unclipped.
//! `scene_broadcast_permitted` is the one admission check, applied at the sender. The relay has
//! no per-connection egress decision: the originating client recognizes and drops its own frame
//! by the relayed `origin` token.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::fmt;

use serde::de::{Deserializer, Error, SeqAccess, Visitor};

/// The most vertices one `ClientMsg::RulerUpdate` may carry; a longer list fails to deserialize
/// (`deserialize_points`) rather than being truncated, so the frame is answered with the generic
/// `BadMessage` and never relayed, and a hostile sender cannot buy broadcast fan-out. A route
/// preview has one vertex per crossed cell at most, and the client caps its own sends to this
/// same number (`RULER_LIMITS.maxPoints` in `@shadowcat/core`, pinned to this constant by a
/// shared fixture), so an honest sender never exceeds it.
pub(crate) const MAX_RULER_POINTS: usize = 256;

/// The longest `ClientMsg::RulerUpdate::origin` token, in bytes; a frame with a longer one is
/// dropped silently.
/// A client's token is one UUID (36 bytes), so an honest sender never exceeds it, and the bound
/// stops the relayed field from inflating every recipient's frame.
pub(crate) const MAX_RULER_ORIGIN_BYTES: usize = 64;

/// The per-user, per-trailing-60s budget the `ClientMsg::RulerUpdate` handler spends against,
/// from its OWN bucket (`WsState::ruler_rate` — live-preview traffic must not starve pings,
/// emotes or chat). 720/min is 12 frames a second: above the most an honest client can send, one
/// update per 200 ms plus the stop that ends each ruler, at most 10 a second from one connection
/// (the scene-tools `RulerSharer` keeps every update at least that far after the previous update
/// or keepalive; a stop does not reset that clock, so an update may follow a stop at once), so a
/// single connection's stop is never the frame dropped; still bounding the broadcast fan-out a
/// hostile sender can buy. The budget is per user, so one user measuring on several connections
/// shares it.
pub(crate) const RULER_RATE_PER_MIN: usize = 720;

/// Deserializes `ClientMsg::RulerUpdate::points`, failing when the sequence holds more than
/// `MAX_RULER_POINTS` vertices. An over-cap list therefore makes the whole frame fail to
/// deserialize as a `ClientMsg`, and a frame that fails to deserialize is answered like every
/// other malformed frame: the sender alone receives a generic `BadMessage`, before any scene
/// lookup, so the reply reveals nothing about any scene. This function bounds what is accepted,
/// not how much of the frame is buffered while it is parsed.
pub(crate) fn deserialize_points<'de, D>(deserializer: D) -> Result<Vec<(f64, f64)>, D::Error>
where
    D: Deserializer<'de>,
{
    /// Collects at most `MAX_RULER_POINTS` vertices from a sequence.
    struct BoundedPoints;

    impl<'de> Visitor<'de> for BoundedPoints {
        type Value = Vec<(f64, f64)>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {MAX_RULER_POINTS} [x, y] vertices")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut points = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_RULER_POINTS));
            while let Some(p) = seq.next_element::<(f64, f64)>()? {
                if points.len() == MAX_RULER_POINTS {
                    return Err(A::Error::invalid_length(MAX_RULER_POINTS + 1, &self));
                }
                points.push(p);
            }
            Ok(points)
        }
    }

    deserializer.deserialize_seq(BoundedPoints)
}

/// Whether `points` is an admissible `ClientMsg::RulerUpdate` payload: every coordinate finite and
/// within `scene::move_exec::MAX_GATE_WALK_COORD` (the magnitude bound every client-supplied scene
/// coordinate obeys — `vfx::validate_bounds` applies it to a one-shot's anchor). The vertex
/// count is bounded at parse time by `deserialize_points`. Pure and I/O-free so it runs before
/// the scene-authorization read.
pub(crate) fn points_admissible(points: &[(f64, f64)]) -> bool {
    let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    points
        .iter()
        .all(|&(x, y)| x.is_finite() && y.is_finite() && x.abs() <= bound && y.abs() <= bound)
}

/// Whether `origin` is an admissible `ClientMsg::RulerUpdate::origin`: at most
/// `MAX_RULER_ORIGIN_BYTES` bytes. Pure and I/O-free so it runs before the scene-authorization
/// read, alongside `points_admissible`.
pub(crate) fn origin_admissible(origin: &str) -> bool {
    origin.len() <= MAX_RULER_ORIGIN_BYTES
}

/// The canonical form of a ruler frame: an update is active only if it is flagged active AND
/// carries at least one vertex, and every other update is the stop signal with no vertices.
/// The server is the definition of a frame's meaning, so recipients never see a stop carrying
/// points nor an "active" ruler with nothing to draw.
pub(crate) fn canonical(points: Vec<(f64, f64)>, active: bool) -> (Vec<(f64, f64)>, bool) {
    if active && !points.is_empty() {
        (points, true)
    } else {
        (Vec::new(), false)
    }
}
