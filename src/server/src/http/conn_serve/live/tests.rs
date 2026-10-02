//! `LeasedBody`'s chunk bound: no data frame it yields exceeds [`EGRESS_FLOOR_BYTES`], the size
//! hint stays exact, and non-data frames pass through.

use std::collections::VecDeque;
use std::net::IpAddr;

use http_body::Body as _;

use super::*;
use crate::http::conn_serve::EGRESS_FLOOR_BYTES;
use crate::http::peer_slots::PeerSlots;

/// A connection's live state over a fresh slot.
fn live() -> Arc<LiveState> {
    let slots = Arc::new(PeerSlots::new(4, 4));
    let ip: IpAddr = "192.0.2.1".parse().unwrap();
    LiveState::new(slots.admit(ip, false).unwrap())
}

/// A body that yields a fixed list of frames, then ends, with a hint derived from what remains.
struct Script {
    /// The frames still to yield.
    frames: VecDeque<Frame<Bytes>>,
}

impl http_body::Body for Script {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Poll::Ready(self.frames.pop_front().map(Ok))
    }

    fn is_end_stream(&self) -> bool {
        self.frames.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        let total: u64 = self
            .frames
            .iter()
            .filter_map(|f| f.data_ref())
            .map(|d| d.len() as u64)
            .sum();
        SizeHint::with_exact(total)
    }
}

/// A leased body over `frames`.
fn leased(frames: Vec<Frame<Bytes>>) -> LeasedBody {
    LeasedBody::new(
        Body::new(Script {
            frames: frames.into(),
        }),
        live().lease(),
    )
}

/// Polls `body` once.
async fn next(body: &mut LeasedBody) -> Option<Result<Frame<Bytes>, axum::Error>> {
    std::future::poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)).await
}

#[tokio::test]
async fn a_one_mebibyte_chunk_is_yielded_as_sixty_four_frames_of_the_floor() {
    let mut body = leased(vec![Frame::data(Bytes::from(vec![7u8; 1 << 20]))]);
    let mut frames = 0usize;
    let mut delivered = 0u64;
    while let Some(frame) = next(&mut body).await {
        let data = frame.unwrap().into_data().unwrap();
        assert_eq!(data.len(), EGRESS_FLOOR_BYTES, "frame {frames}");
        delivered += data.len() as u64;
        frames += 1;
        let hint = body.size_hint();
        assert_eq!(
            hint.exact(),
            Some((1u64 << 20) - delivered),
            "the hint is exact after frame {frames}"
        );
    }
    assert_eq!(frames, 64);
    assert_eq!(delivered, 1 << 20);
}

#[tokio::test]
async fn the_end_of_stream_is_reported_only_after_the_last_pending_slice() {
    let mut body = leased(vec![Frame::data(Bytes::from(vec![
        1u8;
        3 * EGRESS_FLOOR_BYTES
    ]))]);
    assert!(!body.is_end_stream());
    for _ in 0..2 {
        next(&mut body).await.unwrap().unwrap();
        assert!(!body.is_end_stream(), "a slice is still pending");
    }
    next(&mut body).await.unwrap().unwrap();
    assert!(body.is_end_stream());
    assert!(next(&mut body).await.is_none());
}

#[tokio::test]
async fn a_frame_within_the_floor_passes_whole_and_the_remainder_follows() {
    let mut body = leased(vec![
        Frame::data(Bytes::from_static(b"abc")),
        Frame::data(Bytes::new()),
        Frame::data(Bytes::from(vec![2u8; EGRESS_FLOOR_BYTES + 5])),
    ]);
    let lens: Vec<usize> = {
        let mut v = Vec::new();
        while let Some(frame) = next(&mut body).await {
            v.push(frame.unwrap().into_data().unwrap().len());
        }
        v
    };
    assert_eq!(lens, vec![3, 0, EGRESS_FLOOR_BYTES, 5]);
}

#[tokio::test]
async fn trailers_pass_through_after_the_data_slices() {
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-end", axum::http::HeaderValue::from_static("1"));
    let mut body = leased(vec![
        Frame::data(Bytes::from(vec![3u8; EGRESS_FLOOR_BYTES + 1])),
        Frame::trailers(trailers),
    ]);
    assert_eq!(
        next(&mut body)
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .len(),
        EGRESS_FLOOR_BYTES
    );
    assert_eq!(
        next(&mut body)
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .len(),
        1
    );
    let last = next(&mut body).await.unwrap().unwrap();
    assert_eq!(last.trailers_ref().unwrap()["x-end"], "1");
    assert!(next(&mut body).await.is_none());
}

#[tokio::test]
async fn every_slice_shares_the_original_allocation() {
    let big = Bytes::from(vec![9u8; 2 * EGRESS_FLOOR_BYTES]);
    let base = big.as_ptr();
    let mut body = leased(vec![Frame::data(big)]);
    let first = next(&mut body).await.unwrap().unwrap().into_data().unwrap();
    let second = next(&mut body).await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(
        first.as_ptr(),
        base,
        "the first slice is a view, not a copy"
    );
    assert_eq!(
        second.as_ptr(),
        base.wrapping_add(EGRESS_FLOOR_BYTES),
        "the second slice is a view, not a copy"
    );
}
