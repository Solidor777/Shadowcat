//! This module is the blocking half of the derivative upgrade sweep.
//!
//! For one stored audio asset it decides which of its Opus siblings (`WEBM_SUFFIX`, `OPUS_SUFFIX`)
//! are stale, stages the replacement files beside them and swaps them in.
//!
//! A marker separates a stale sibling from a current one. A WebM sibling is stale when its head
//! carries no `WEBM_WRITING_APP` marker; an unmarked WebM also lacks `Duration`, `SeekHead`, `Cues`
//! and `DiscardPadding`. An Ogg sibling is stale when its vendor string is not `OPUS_VENDOR_MARKER`
//! and `loss_range` holds for its pre-skip and valid-sample count. An unmarked Ogg in the loss
//! range has no lookahead flush and is missing the last samples of the audio. An asset is selected
//! when either sibling is stale.
//!
//! The repair depends on which sibling is stale and what is stored. An Ogg in the loss range cannot
//! be repaired from its own packets, because the missing samples were never encoded. With the
//! stored original present, a decode to the same length as the Ogg re-encodes every present sibling
//! from it (`Kind::Retranscode`), and a decode to a different length leaves the asset alone
//! (`Plan::Unrepairable`). With the original absent, a stale WebM is remuxed to its decodable
//! length (`Kind::RemuxToDecodableLength`) and the Ogg is left as it is, while a WebM that is
//! already current or absent leaves nothing to write (`Plan::Unrepairable`).
//!
//! When only the WebM is stale, an Ogg beside it supplies the packets for a remux (`Kind::Remux`),
//! which keeps the two siblings one encode. A stale WebM with no Ogg is re-encoded from the
//! original (`Kind::Retranscode`), and with no original either the asset is left alone
//! (`Plan::Unrepairable`). `plan` returns `Plan::Current` when neither sibling is stale and an
//! `Err` when a sibling is unreadable or a decode, mux or staged write fails;
//! `UpgradeReport.failed` counts the `Err`.
//!
//! The sweep never writes the database, never bumps a version and never goes through
//! `reconvert`, `commit_replacement` or `move_asset_files` (which deletes a final sibling that has
//! no staged match). Each swap is a rename of a staged temporary file over the sibling, WebM
//! first and Ogg last, because the loss predicate reads the Ogg: an interrupted sweep leaves the
//! asset selected on the next pass and never a fixed Ogg beside an unfixed WebM.

use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::webm::{head_is_current, mux_webm, WEBM_HEAD_BYTES};
use super::{
    mux_ogg, transcode, with_suffix, OpusStream, OPUS_FRAME_SAMPLES, OPUS_SUFFIX,
    OPUS_VENDOR_MARKER, WEBM_SUFFIX,
};
use crate::trash::Trash;

/// Bytes read from the start of the Ogg sibling (`OPUS_SUFFIX`) to cover its `OpusHead` and
/// `OpusTags` pages.
const OGG_HEAD_BYTES: usize = 8192;

/// Bytes read from the end of the Ogg sibling (`OPUS_SUFFIX`): more than the largest Ogg page
/// (65,307 bytes), so the final page's header is always inside the window.
const OGG_TAIL_BYTES: u64 = 70_000;

/// The Ogg facts the selection needs, read without scanning the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OggInfo {
    /// Channel count from `OpusHead`.
    pub(crate) channels: u16,
    /// Pre-skip from `OpusHead`, samples.
    pub(crate) pre_skip: u64,
    /// Input sample rate from `OpusHead`.
    pub(crate) input_rate: u32,
    /// The `OpusTags` vendor string.
    pub(crate) vendor: String,
    /// The final page's granule position: `pre_skip + total_valid` (RFC 7845).
    pub(crate) final_granule: u64,
}

impl OggInfo {
    /// The exact count of valid samples per channel: the final granule less the pre-skip.
    pub(crate) fn total_valid(&self) -> Option<u64> {
        self.final_granule.checked_sub(self.pre_skip)
    }

    /// Whether the vendor string is not `OPUS_VENDOR_MARKER` and `loss_range` holds for the
    /// pre-skip and `total_valid`. A marked file carries the flush and is never in the range.
    pub(crate) fn in_loss_range(&self) -> bool {
        if self.vendor == OPUS_VENDOR_MARKER {
            return false;
        }
        let Some(valid) = self.total_valid() else {
            return false;
        };
        loss_range(self.pre_skip, valid)
    }
}

/// Whether an unflushed encode of `total_valid` samples with `pre_skip` lookahead loses its tail.
/// The encoder emits each sample `pre_skip` samples late, so the `ceil(total_valid / 960)` frames
/// of an unflushed stream cover `frames * 960 - pre_skip` audible samples, fewer than `total_valid`
/// exactly when `frames * 960 < pre_skip + total_valid`.
pub(crate) fn loss_range(pre_skip: u64, total_valid: u64) -> bool {
    total_valid.div_ceil(OPUS_FRAME_SAMPLES) * OPUS_FRAME_SAMPLES < pre_skip + total_valid
}

/// Reads the head of `path` (the `OpusHead` and `OpusTags` packets) and its final granule from the
/// file's last page: two bounded reads, no stream scan.
pub(crate) fn read_ogg_info(path: &Path) -> io::Result<OggInfo> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = vec![0u8; OGG_HEAD_BYTES.min(len as usize)];
    file.read_exact(&mut head)?;
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(head));
    let opus_head = reader
        .read_packet_expected()
        .map_err(|e| invalid(&format!("OpusHead: {e}")))?;
    let tags = reader
        .read_packet_expected()
        .map_err(|e| invalid(&format!("OpusTags: {e}")))?;
    let data = &opus_head.data;
    if data.len() < 19 || &data[..8] != b"OpusHead" {
        return Err(invalid("not an OpusHead"));
    }
    let tag_data = &tags.data;
    if tag_data.len() < 12 || &tag_data[..8] != b"OpusTags" {
        return Err(invalid("not an OpusTags"));
    }
    let vendor_len = u32::from_le_bytes(tag_data[8..12].try_into().expect("4 bytes")) as usize;
    let vendor = tag_data
        .get(12..12 + vendor_len)
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .ok_or_else(|| invalid("OpusTags vendor overruns the packet"))?;

    let tail_start = len.saturating_sub(OGG_TAIL_BYTES);
    file.seek(SeekFrom::Start(tail_start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let final_granule = last_page_granule(&tail).ok_or_else(|| invalid("no final Ogg page"))?;
    Ok(OggInfo {
        channels: data[9] as u16,
        pre_skip: u16::from_le_bytes([data[10], data[11]]) as u64,
        input_rate: u32::from_le_bytes(data[12..16].try_into().expect("4 bytes")),
        vendor,
        final_granule,
    })
}

/// The granule position of the last Ogg page whose header lies in `tail`: the last `OggS` capture
/// pattern followed by stream-structure version 0 and a header-type byte with the end-of-stream
/// bit, so payload bytes that happen to spell `OggS` do not match.
fn last_page_granule(tail: &[u8]) -> Option<u64> {
    let mut end = tail.len();
    while let Some(pos) = tail[..end].windows(4).rposition(|w| w == b"OggS") {
        if let Some(header) = tail.get(pos..pos + 14) {
            let end_of_stream = header[5] & 0x04 != 0;
            if header[4] == 0 && end_of_stream {
                return Some(u64::from_le_bytes(
                    header[6..14].try_into().expect("8 bytes"),
                ));
            }
        }
        end = pos;
    }
    None
}

/// Every audio packet of an Ogg/Opus file with the header facts, read whole (BLOCKING).
fn read_ogg_stream(path: &Path) -> Result<(OpusStream, OggInfo), String> {
    let info = read_ogg_info(path).map_err(|e| e.to_string())?;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
    let _head = reader.read_packet_expected().map_err(|e| e.to_string())?;
    let _tags = reader.read_packet_expected().map_err(|e| e.to_string())?;
    let mut packets = Vec::new();
    while let Some(packet) = reader.read_packet().map_err(|e| e.to_string())? {
        packets.push(packet.data);
    }
    if packets.is_empty() {
        return Err("no audio packets".to_string());
    }
    let total_valid = info
        .total_valid()
        .ok_or_else(|| "final granule below the pre-skip".to_string())?;
    Ok((
        OpusStream {
            packets,
            pre_skip: u16::try_from(info.pre_skip).map_err(|_| "pre-skip too large".to_string())?,
            total_valid,
        },
        info,
    ))
}

/// The identity of a file at one moment: its length and modification time. A sibling that is
/// replaced between staging and swapping (an asset replace renames new siblings in) differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    /// Length in bytes.
    len: u64,
    /// Modification time.
    modified: Option<SystemTime>,
}

/// The fingerprint of `path`, or `None` when it does not exist.
fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let metadata = fs::metadata(path).ok()?;
    Some(Fingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// How a staged replacement was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The existing Opus packets were copied into the current WebM layout.
    Remux,
    /// The WebM was remuxed from an Ogg whose lost tail cannot be recovered: written with no
    /// padding and a `Duration` of the decodable length, the Ogg left as it is.
    RemuxToDecodableLength,
    /// The stored original was decoded and encoded again; the siblings come from that one encode.
    Retranscode,
}

/// One staged replacement file.
#[derive(Debug)]
pub(crate) struct Staged {
    /// The temporary file holding the new bytes, beside the target.
    tmp: PathBuf,
    /// The sibling it replaces.
    target: PathBuf,
    /// The target as it was when its packets were read.
    expect: Fingerprint,
}

/// A decision for one asset.
#[derive(Debug)]
pub(crate) enum Plan {
    /// Both siblings are current: nothing selected the asset.
    Current,
    /// Selected, but no write can improve the files; `reason` says why. The asset is selected
    /// again by the next pass, which reaches the same answer.
    Unrepairable(&'static str),
    /// Staged files, in swap order (WebM before Ogg).
    Write {
        /// How the files were made.
        kind: Kind,
        /// The staged replacements.
        staged: Vec<Staged>,
    },
}

/// The outcome of swapping a plan in.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// Every staged file replaced its target.
    Done(usize),
    /// A target no longer matches what was staged against (replaced or deleted), or a rename
    /// failed: the staged files move into the trash and the asset is selected again by the next pass.
    Skipped(String),
}

/// A staged file's temporary path beside `target`.
fn tmp_beside(canonical: &Path) -> PathBuf {
    with_suffix(canonical, &format!(".{}.tmp", uuid::Uuid::new_v4()))
}

/// Writes `bytes` to a new temporary file beside `canonical` and returns it staged against
/// `target`.
fn stage(canonical: &Path, target: PathBuf, bytes: &[u8]) -> io::Result<Staged> {
    let expect = fingerprint(&target)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "sibling vanished"))?;
    let tmp = tmp_beside(canonical);
    fs::write(&tmp, bytes)?;
    Ok(Staged {
        tmp,
        target,
        expect,
    })
}

/// Moves the staged temporary files of a plan that is not applied into `trash`.
pub(crate) fn discard(staged: &[Staged], trash: &Trash) {
    for s in staged {
        let _ = trash.discard_sync(&s.tmp);
    }
}

/// Decides and stages the upgrade of the Opus siblings beside `canonical` (BLOCKING: decode and
/// encode are CPU-bound, so the caller runs it on the blocking pool). Writes only temporary files.
///
/// Selects the asset when either sibling is stale, as the module doc defines.
pub(crate) fn plan(canonical: &Path, trash: &Trash) -> Result<Plan, String> {
    let webm_path = with_suffix(canonical, WEBM_SUFFIX);
    let ogg_path = with_suffix(canonical, OPUS_SUFFIX);
    let has_webm = webm_path.is_file();
    let has_ogg = ogg_path.is_file();
    let webm_stale = has_webm && !webm_head_is_current(&webm_path).map_err(|e| e.to_string())?;
    let ogg_info = if has_ogg {
        Some(read_ogg_info(&ogg_path).map_err(|e| format!("{OPUS_SUFFIX}: {e}"))?)
    } else {
        None
    };
    let ogg_loss = ogg_info.as_ref().is_some_and(OggInfo::in_loss_range);
    if !webm_stale && !ogg_loss {
        return Ok(Plan::Current);
    }
    let original_decodes = canonical.is_file();

    if ogg_loss {
        if !original_decodes {
            // The lost tail was never encoded into any stored file: remux what exists.
            if !webm_stale {
                return Ok(Plan::Unrepairable(
                    "tail loss, original missing, webm already remuxed",
                ));
            }
            let (mut stream, _) = read_ogg_stream(&ogg_path)?;
            let info = ogg_info.as_ref().expect("ogg_loss implies an ogg");
            stream.total_valid = (stream.packets.len() as u64) * OPUS_FRAME_SAMPLES - info.pre_skip;
            let bytes = mux_webm(&stream, info.channels, info.input_rate)?;
            let staged = stage(canonical, webm_path, &bytes).map_err(|e| e.to_string())?;
            return Ok(Plan::Write {
                kind: Kind::RemuxToDecodableLength,
                staged: vec![staged],
            });
        }
        let t = transcode(canonical).map_err(|note| format!("original: {note}"))?;
        let old_valid = ogg_info.as_ref().and_then(OggInfo::total_valid);
        if old_valid != Some(t.stream.total_valid) {
            return Ok(Plan::Unrepairable(
                "original decodes to a different length than the stored ogg",
            ));
        }
        let mut staged = Vec::new();
        let result = (|| -> Result<(), String> {
            if has_webm {
                let bytes = mux_webm(&t.stream, t.channels, t.source_rate)?;
                staged
                    .push(stage(canonical, webm_path.clone(), &bytes).map_err(|e| e.to_string())?);
            }
            let bytes = mux_ogg(&t.stream, t.channels, t.source_rate)?;
            staged.push(stage(canonical, ogg_path.clone(), &bytes).map_err(|e| e.to_string())?);
            Ok(())
        })();
        if let Err(e) = result {
            discard(&staged, trash);
            return Err(e);
        }
        return Ok(Plan::Write {
            kind: Kind::Retranscode,
            staged,
        });
    }

    // Only the WebM is stale.
    if has_ogg {
        let (stream, info) = read_ogg_stream(&ogg_path)?;
        let bytes = mux_webm(&stream, info.channels, info.input_rate)?;
        let staged = stage(canonical, webm_path, &bytes).map_err(|e| e.to_string())?;
        return Ok(Plan::Write {
            kind: Kind::Remux,
            staged: vec![staged],
        });
    }
    // WebM only: an unmarked WebM carries no valid-sample count, so the original is decoded again.
    if !original_decodes {
        return Ok(Plan::Unrepairable("webm only and the original is missing"));
    }
    let t = transcode(canonical).map_err(|note| format!("original: {note}"))?;
    let bytes = mux_webm(&t.stream, t.channels, t.source_rate)?;
    let staged = stage(canonical, webm_path, &bytes).map_err(|e| e.to_string())?;
    Ok(Plan::Write {
        kind: Kind::Retranscode,
        staged: vec![staged],
    })
}

/// Whether the head of the WebM sibling at `path` carries the current writing-app marker.
fn webm_head_is_current(path: &Path) -> io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut head = Vec::with_capacity(WEBM_HEAD_BYTES);
    (&mut file)
        .take(WEBM_HEAD_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok(head_is_current(&head))
}

/// The rename `apply` performs for each staged file: `std::fs::rename` in production. A swap is a
/// rename over the target; the target's open readers keep the old bytes.
pub(crate) type Rename = fn(&Path, &Path) -> io::Result<()>;

/// The production `Rename`: `std::fs::rename`, which replaces the target atomically where the
/// platform allows it.
pub(crate) fn rename_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Swaps the staged files of `plan` over their targets in order, after checking each target is
/// still the file the packets were read from. A mismatch or a failed rename stops the swap, moves
/// every staged file that was not renamed into `trash` and reports `Applied::Skipped`; the targets already
/// renamed stay (WebM first, so the asset is selected again while its Ogg is unfixed).
///
/// The fingerprint check and the rename are two steps, so the check only holds if nothing else
/// replaces or deletes the target between them. `apply` takes no lock itself: the caller
/// (`audio_upgrade::upgrade_one`) holds the write side of the write barrier around the call, which
/// excludes every asset replace, delete and reconvert.
pub(crate) fn apply(staged: &[Staged], rename: Rename, trash: &Trash) -> Applied {
    for (done, s) in staged.iter().enumerate() {
        if fingerprint(&s.target) != Some(s.expect) {
            discard(&staged[done..], trash);
            return Applied::Skipped(format!("{} changed since it was read", s.target.display()));
        }
        if let Err(e) = rename(&s.tmp, &s.target) {
            discard(&staged[done..], trash);
            return Applied::Skipped(format!("rename over {}: {e}", s.target.display()));
        }
    }
    Applied::Done(staged.len())
}

#[cfg(test)]
mod tests;
