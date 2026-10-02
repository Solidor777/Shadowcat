//! The HTTP/2 frame walker the egress bounds read the wire with. One [`FrameParser`] type, two
//! instances per HTTP/2 connection: the WRITE instance walks what the writer accepted (counting
//! DATA payload for the connection unit and reporting which frames ended which streams), and the
//! READ instance walks what the codec is handed (reporting request HEADERS and RST_STREAM).
//!
//! A frame is `9` header bytes (24-bit length, type, flags, 31-bit stream id) and a payload; the
//! walker collects a header in whatever pieces the bytes arrive (split across writes and across
//! `IoSlice`s), then skips the payload by arithmetic, so its cost is per frame and never per byte.
//! Frame layout: RFC 9113 section 4.1.

use super::super::H2_PREFACE_LEN;

/// The length of an HTTP/2 frame header (RFC 9113 section 4.1).
const FRAME_HEADER_LEN: usize = 9;

/// The type code of a DATA frame (RFC 9113 section 6.1).
pub(super) const FRAME_TYPE_DATA: u8 = 0;

/// The type code of a HEADERS frame (RFC 9113 section 6.2).
pub(super) const FRAME_TYPE_HEADERS: u8 = 1;

/// The type code of an RST_STREAM frame (RFC 9113 section 6.4).
pub(super) const FRAME_TYPE_RST_STREAM: u8 = 3;

/// The type code of a GOAWAY frame (RFC 9113 section 6.8).
const FRAME_TYPE_GOAWAY: u8 = 7;

/// The type code of a CONTINUATION frame (RFC 9113 section 6.10).
pub(super) const FRAME_TYPE_CONTINUATION: u8 = 9;

/// The END_STREAM flag of DATA and HEADERS frames (RFC 9113 sections 6.1 and 6.2).
pub(super) const FLAG_END_STREAM: u8 = 0x1;

/// The END_HEADERS flag of HEADERS and CONTINUATION frames (RFC 9113 sections 6.2 and 6.10).
pub(super) const FLAG_END_HEADERS: u8 = 0x4;

/// The stream-id field is 31 bits; the top bit is reserved (RFC 9113 section 4.1).
const STREAM_ID_MASK: u32 = 0x7fff_ffff;

/// What walking the bytes found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameEvent {
    /// `n` DATA payload bytes were consumed. A PADDED DATA frame counts its whole length field:
    /// the server writes no padding, so a padded frame is overcounted by at most 256 bytes.
    Data(u64),
    /// A frame's 9-byte header was completed.
    Start {
        /// The frame type.
        kind: u8,
        /// The stream the frame belongs to (`0` for connection frames).
        sid: u32,
    },
    /// A frame's last byte was consumed (for a frame with no payload, the last byte of its header).
    End {
        /// The frame type.
        kind: u8,
        /// The frame's flags.
        flags: u8,
        /// The stream the frame belongs to.
        sid: u32,
    },
    /// A GOAWAY frame was consumed; carries its last-stream-id. Reported just before its `End`.
    GoAway(u32),
}

/// Walks HTTP/2 frames in whatever pieces they arrive.
///
/// INVARIANT: the first byte fed is the first byte of a frame, or of the client preface when built
/// with [`FrameParser::with_preface`]. The WRITE side sees plaintext frames starting with the
/// server's SETTINGS because the parser sits after the TLS layer; the READ side starts in the
/// 24-byte preface because the preface is replayed to the codec as bytes.
#[derive(Default)]
pub(super) struct FrameParser {
    /// Preface bytes still to skip.
    preface_left: usize,
    /// The frame header bytes collected so far.
    header: [u8; FRAME_HEADER_LEN],
    /// How many `header` bytes are filled, `0..FRAME_HEADER_LEN`.
    filled: usize,
    /// Payload bytes of the current frame still to skip.
    left: u32,
    /// The current frame's payload length.
    total: u32,
    /// The current frame's type.
    kind: u8,
    /// The current frame's flags.
    flags: u8,
    /// The current frame's stream id.
    sid: u32,
    /// The first four payload bytes of a GOAWAY frame: its last-stream-id.
    goaway: [u8; 4],
    /// Loop passes taken, so a test can assert the cost is per frame and not per byte.
    #[cfg(test)]
    steps: usize,
}

impl FrameParser {
    /// A parser whose first byte starts a frame.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// A parser whose first [`H2_PREFACE_LEN`] bytes are the client connection preface.
    pub(super) fn with_preface() -> Self {
        Self {
            preface_left: H2_PREFACE_LEN,
            ..Self::default()
        }
    }

    /// Consumes `bytes`, the next bytes of the stream, and reports what they completed to `on`.
    pub(super) fn feed(&mut self, mut bytes: &[u8], on: &mut impl FnMut(FrameEvent)) {
        while !bytes.is_empty() {
            #[cfg(test)]
            {
                self.steps += 1;
            }
            if self.preface_left > 0 {
                let skip = bytes.len().min(self.preface_left);
                self.preface_left -= skip;
                bytes = &bytes[skip..];
                continue;
            }
            if self.left > 0 {
                let skip = bytes.len().min(self.left as usize);
                self.consume_payload(&bytes[..skip], on);
                self.left -= skip as u32;
                bytes = &bytes[skip..];
                if self.left == 0 {
                    self.finish(on);
                }
                continue;
            }
            let take = (FRAME_HEADER_LEN - self.filled).min(bytes.len());
            self.header[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
            self.filled += take;
            bytes = &bytes[take..];
            if self.filled == FRAME_HEADER_LEN {
                self.begin(on);
            }
        }
    }

    /// The header is complete: starts the frame, and finishes it at once when it has no payload.
    fn begin(&mut self, on: &mut impl FnMut(FrameEvent)) {
        self.total = u32::from_be_bytes([0, self.header[0], self.header[1], self.header[2]]);
        self.left = self.total;
        self.kind = self.header[3];
        self.flags = self.header[4];
        self.sid = u32::from_be_bytes([
            self.header[5],
            self.header[6],
            self.header[7],
            self.header[8],
        ]) & STREAM_ID_MASK;
        on(FrameEvent::Start {
            kind: self.kind,
            sid: self.sid,
        });
        if self.left == 0 {
            self.finish(on);
        }
    }

    /// Handles a run of payload bytes of the current frame.
    fn consume_payload(&mut self, payload: &[u8], on: &mut impl FnMut(FrameEvent)) {
        if self.kind == FRAME_TYPE_DATA {
            on(FrameEvent::Data(payload.len() as u64));
        } else if self.kind == FRAME_TYPE_GOAWAY {
            let offset = (self.total - self.left) as usize;
            for (i, byte) in payload
                .iter()
                .take(4usize.saturating_sub(offset))
                .enumerate()
            {
                self.goaway[offset + i] = *byte;
            }
        }
    }

    /// The frame's last byte was consumed.
    fn finish(&mut self, on: &mut impl FnMut(FrameEvent)) {
        if self.kind == FRAME_TYPE_GOAWAY && self.total >= 4 {
            on(FrameEvent::GoAway(
                u32::from_be_bytes(self.goaway) & STREAM_ID_MASK,
            ));
        }
        on(FrameEvent::End {
            kind: self.kind,
            flags: self.flags,
            sid: self.sid,
        });
        self.filled = 0;
    }
}

#[cfg(test)]
mod tests;
