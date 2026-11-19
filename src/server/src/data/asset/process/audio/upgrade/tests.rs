//! These tests cover the upgrade of stored Opus siblings written by an unmarked encoder and muxer.
//!
//! The fixtures are the output of the unmarked muxer (no marker, `Duration`, `SeekHead`, `Cues` or
//! `DiscardPadding`) for a 48 kHz mono sine of `12 * 960 + residue` valid samples, `residue` being
//! the remainder of the valid count modulo one 960-sample frame. Residues 0 and 649 through 959
//! are the loss range of a 312-sample lookahead, and 648 is the last residue outside it; the
//! fixtures cover residues 0, 648, 649 and 959.

use std::path::{Path, PathBuf};

use super::super::tests::synth_wav_440hz_at;
use super::super::webm::tests::{discard_paddings, libopus_decode, tail_rms, webm_trim};
use super::*;

/// Where the committed unmarked-layout fixtures live.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/legacy-opus");

/// Valid samples of the fixture with the given residue.
fn valid_of(residue: usize) -> u64 {
    12 * 960 + residue as u64
}

/// The bytes of an unmarked-layout fixture.
fn fixture(residue: usize, suffix: &str) -> Vec<u8> {
    std::fs::read(Path::new(FIXTURES).join(format!("residue-{residue}{suffix}"))).unwrap()
}

/// One asset's files in a temporary store.
struct Stored {
    /// Keeps the directory alive.
    _dir: tempfile::TempDir,
    /// The canonical path (the stored original, when present).
    canonical: PathBuf,
}

impl Stored {
    /// The Ogg sibling path (`OPUS_SUFFIX`).
    fn ogg(&self) -> PathBuf {
        with_suffix(&self.canonical, OPUS_SUFFIX)
    }

    /// The WebM sibling path (`WEBM_SUFFIX`).
    fn webm(&self) -> PathBuf {
        with_suffix(&self.canonical, WEBM_SUFFIX)
    }

    /// Temporary files left beside the asset.
    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(self.canonical.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }
}

/// Stores the unmarked-layout siblings of `residue` that `ogg` / `webm` ask for, and the original
/// WAV when `original` is set.
fn stored(residue: usize, ogg: bool, webm: bool, original: bool) -> Stored {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("asset");
    if ogg {
        std::fs::write(
            with_suffix(&canonical, OPUS_SUFFIX),
            fixture(residue, ".opus.ogg"),
        )
        .unwrap();
    }
    if webm {
        std::fs::write(
            with_suffix(&canonical, WEBM_SUFFIX),
            fixture(residue, ".opus.webm"),
        )
        .unwrap();
    }
    if original {
        let wav = synth_wav_440hz_at(48_000, valid_of(residue) as f64 / 48_000.0);
        std::fs::write(&canonical, wav).unwrap();
    }
    Stored {
        _dir: dir,
        canonical,
    }
}

/// Plans `asset`, expecting a write, and applies it with the real rename.
fn upgrade(asset: &Stored) -> (Kind, usize) {
    let Plan::Write { kind, staged } = plan(&asset.canonical).unwrap() else {
        panic!("expected a write");
    };
    let count = staged.len();
    assert_eq!(apply(&staged, rename_file), Applied::Done(count));
    (kind, count)
}

/// Whether the WebM at `path` carries the current writing-app marker in its head.
fn is_current(path: &Path) -> bool {
    let bytes = std::fs::read(path).unwrap();
    head_is_current(&bytes[..bytes.len().min(WEBM_HEAD_BYTES)])
}

/// The audible sample count and tail level of a WebM, decoding the way a browser does: drop
/// `CodecDelay` from the front and `DiscardPadding` from the end.
fn webm_audible(path: &Path) -> (u64, f32, f64, bool) {
    let bytes = std::fs::read(path).unwrap();
    let (packets, delay, padding, duration_ms) = webm_trim(&bytes);
    let decoded = libopus_decode(&packets);
    let audible = decoded.len() as u64 - delay - padding;
    let kept = &decoded[delay as usize..(delay + audible) as usize];
    let has_padding = !discard_paddings(&bytes).0.is_empty();
    (audible, tail_rms(kept, 100), duration_ms, has_padding)
}

/// The loss range of a 312-sample lookahead over every residue is exactly 0 and 649 to 959.
#[test]
fn the_loss_range_is_residue_zero_and_649_to_959() {
    for residue in 0..960u64 {
        let valid = 12 * 960 + residue;
        assert_eq!(
            loss_range(312, valid),
            residue == 0 || residue >= 649,
            "residue {residue}"
        );
    }
}

/// The two small reads recover the pre-skip, the vendor string and the final granule, and the
/// loss-range predicate follows them.
#[test]
fn the_ogg_info_of_an_unmarked_file_names_its_loss_range() {
    for residue in [0, 648, 649, 959] {
        let asset = stored(residue, true, false, false);
        let info = read_ogg_info(&asset.ogg()).unwrap();
        assert_eq!(info.pre_skip, 312);
        assert_eq!(info.vendor, "");
        assert_eq!(info.final_granule, 312 + valid_of(residue));
        assert_eq!(info.channels, 1);
        assert_eq!(info.input_rate, 48_000);
        assert_eq!(
            info.in_loss_range(),
            residue == 0 || residue >= 649,
            "residue {residue}"
        );
    }
}

/// Outside the loss range the WebM is remuxed from the Ogg's packets: the Ogg is untouched, no
/// padding element is written (the padding is exactly zero at residue 648) and a browser-style
/// decode yields exactly the valid count.
#[test]
fn residue_648_is_remuxed_and_its_ogg_is_byte_identical() {
    let asset = stored(648, true, true, true);
    let before = std::fs::read(asset.ogg()).unwrap();
    let (kind, count) = upgrade(&asset);
    assert_eq!((kind, count), (Kind::Remux, 1));
    assert_eq!(
        std::fs::read(asset.ogg()).unwrap(),
        before,
        "ogg byte-identical"
    );
    let (audible, rms, duration_ms, has_padding) = webm_audible(&asset.webm());
    assert_eq!(audible, valid_of(648));
    assert!(rms > 0.2);
    assert_eq!(duration_ms, valid_of(648) as f64 * 1000.0 / 48_000.0);
    assert!(!has_padding, "no padding element at residue 648");
    assert!(is_current(&asset.webm()));
    assert!(matches!(plan(&asset.canonical).unwrap(), Plan::Current));
    assert!(asset.leftovers().is_empty());
}

/// In the loss range both siblings are re-encoded from the stored original in one encode, WebM
/// swapped first and Ogg last, and each decodes to exactly the valid count with its tail intact.
#[test]
fn loss_range_residues_re_encode_both_siblings_from_one_encode() {
    for residue in [0, 649, 959] {
        let asset = stored(residue, true, true, true);
        let Plan::Write { kind, staged } = plan(&asset.canonical).unwrap() else {
            panic!("residue {residue}: expected a write");
        };
        assert_eq!(kind, Kind::Retranscode, "residue {residue}");
        let targets: Vec<_> = staged.iter().map(|s| s.target.clone()).collect();
        assert_eq!(
            targets,
            vec![asset.webm(), asset.ogg()],
            "WebM first, Ogg last"
        );
        assert_eq!(apply(&staged, rename_file), Applied::Done(2));

        let (audible, rms, duration_ms, has_padding) = webm_audible(&asset.webm());
        assert_eq!(audible, valid_of(residue), "webm residue {residue}");
        assert!(rms > 0.2, "webm tail audible, residue {residue}");
        assert_eq!(duration_ms, valid_of(residue) as f64 * 1000.0 / 48_000.0);
        assert!(
            has_padding,
            "the flush frames leave a positive padding, residue {residue}"
        );

        let info = read_ogg_info(&asset.ogg()).unwrap();
        assert_eq!(info.vendor, OPUS_VENDOR_MARKER);
        assert_eq!(info.final_granule, info.pre_skip + valid_of(residue));
        assert!(!info.in_loss_range());
        let (ogg_stream, _) = read_ogg_stream(&asset.ogg()).unwrap();
        let decoded = libopus_decode(&ogg_stream.packets);
        let kept = &decoded[info.pre_skip as usize..info.final_granule as usize];
        assert_eq!(kept.len() as u64, valid_of(residue));
        assert!(
            tail_rms(kept, 100) > 0.2,
            "ogg tail audible, residue {residue}"
        );

        let webm_packets = webm_trim(&std::fs::read(asset.webm()).unwrap()).0;
        assert_eq!(
            webm_packets, ogg_stream.packets,
            "one encode, residue {residue}"
        );
        assert!(matches!(plan(&asset.canonical).unwrap(), Plan::Current));
        assert!(asset.leftovers().is_empty());
    }
}

/// A sweep interrupted between the two swaps (the Ogg's rename fails) leaves the WebM fixed and
/// the Ogg intact, so the asset is selected again, and never a fixed Ogg beside an unfixed WebM.
#[test]
fn an_interrupted_sweep_is_selected_again_and_never_fixes_the_ogg_first() {
    fn fail_the_ogg(from: &Path, to: &Path) -> io::Result<()> {
        if to.to_string_lossy().ends_with(OPUS_SUFFIX) {
            return Err(io::Error::other("injected"));
        }
        rename_file(from, to)
    }
    let asset = stored(649, true, true, true);
    let old_ogg = std::fs::read(asset.ogg()).unwrap();
    let Plan::Write { staged, .. } = plan(&asset.canonical).unwrap() else {
        panic!("expected a write");
    };
    assert!(matches!(apply(&staged, fail_the_ogg), Applied::Skipped(_)));
    assert!(is_current(&asset.webm()), "the WebM is swapped first");
    assert_eq!(
        std::fs::read(asset.ogg()).unwrap(),
        old_ogg,
        "the Ogg is intact"
    );
    assert!(
        asset.leftovers().is_empty(),
        "the unswapped staged file is removed"
    );
    let Plan::Write { kind, .. } = plan(&asset.canonical).unwrap() else {
        panic!("the asset must be selected again");
    };
    assert_eq!(kind, Kind::Retranscode);
    let (_, count) = upgrade(&asset);
    assert_eq!(count, 2);
    assert!(matches!(plan(&asset.canonical).unwrap(), Plan::Current));
}

/// A loss-range asset whose original is gone cannot be repaired: its WebM is remuxed to the length
/// a decoder can produce (no padding, a `Duration` of that length), its Ogg is left alone, and the
/// next pass finds nothing more to do.
#[test]
fn a_loss_range_asset_without_its_original_is_remuxed_to_its_decodable_length() {
    let asset = stored(649, true, true, false);
    let old_ogg = std::fs::read(asset.ogg()).unwrap();
    let (kind, count) = upgrade(&asset);
    assert_eq!((kind, count), (Kind::RemuxToDecodableLength, 1));
    let decodable = 13 * 960 - 312;
    let (audible, _, duration_ms, has_padding) = webm_audible(&asset.webm());
    assert_eq!(audible, decodable);
    assert_eq!(duration_ms, decodable as f64 * 1000.0 / 48_000.0);
    assert!(!has_padding);
    assert_eq!(std::fs::read(asset.ogg()).unwrap(), old_ogg);
    assert!(matches!(
        plan(&asset.canonical).unwrap(),
        Plan::Unrepairable(_)
    ));
}

/// A WebM-only asset has no valid-sample count in its file, so it is re-encoded from the original
/// and no Ogg sibling appears; without the original there is nothing to do.
#[test]
fn a_webm_only_asset_is_re_encoded_from_its_original_and_gains_no_ogg() {
    let asset = stored(648, false, true, true);
    let (kind, count) = upgrade(&asset);
    assert_eq!((kind, count), (Kind::Retranscode, 1));
    assert!(!asset.ogg().exists());
    assert_eq!(webm_audible(&asset.webm()).0, valid_of(648));

    let orphan = stored(648, false, true, false);
    assert!(matches!(
        plan(&orphan.canonical).unwrap(),
        Plan::Unrepairable(_)
    ));
}

/// An Ogg-only asset in the loss range has only its Ogg re-encoded (no WebM is added); one outside
/// the range is not selected at all.
#[test]
fn an_ogg_only_asset_is_re_encoded_only_inside_the_loss_range() {
    let asset = stored(959, true, false, true);
    let (kind, count) = upgrade(&asset);
    assert_eq!((kind, count), (Kind::Retranscode, 1));
    assert!(!asset.webm().exists());
    let info = read_ogg_info(&asset.ogg()).unwrap();
    assert_eq!(info.final_granule, info.pre_skip + valid_of(959));
    assert_eq!(info.vendor, OPUS_VENDOR_MARKER);

    let outside = stored(648, true, false, true);
    assert!(matches!(plan(&outside.canonical).unwrap(), Plan::Current));
}

/// A target replaced between staging and swapping (an asset replace renames new siblings in) is
/// not overwritten with the stale remux, and the staged file is removed.
#[test]
fn a_sibling_replaced_after_staging_is_not_overwritten() {
    let asset = stored(648, true, true, true);
    let Plan::Write { staged, .. } = plan(&asset.canonical).unwrap() else {
        panic!("expected a write");
    };
    std::fs::write(asset.webm(), b"a newer derivative of a different length").unwrap();
    assert!(matches!(apply(&staged, rename_file), Applied::Skipped(_)));
    assert_eq!(
        std::fs::read(asset.webm()).unwrap(),
        b"a newer derivative of a different length"
    );
    assert!(asset.leftovers().is_empty());
}

/// A reader holding the target across the swap sees either the new file (the swap went through
/// and the reader keeps the old bytes) or the old file intact (the platform refused the rename and
/// the swap was skipped); either way the sweep terminates and leaves no staged file behind. On
/// Windows the swap goes through (`Applied::Done`): `fs::File::open` shares the file for delete, so
/// the rename replaces the target while the reader keeps the old bytes.
#[test]
fn a_reader_held_across_the_swap_leaves_the_old_or_the_new_file_intact() {
    let asset = stored(648, true, true, true);
    let old = std::fs::read(asset.webm()).unwrap();
    let mut reader = fs::File::open(asset.webm()).unwrap();
    let Plan::Write { staged, .. } = plan(&asset.canonical).unwrap() else {
        panic!("expected a write");
    };
    match apply(&staged, rename_file) {
        Applied::Done(_) => {
            let mut held = Vec::new();
            reader.read_to_end(&mut held).unwrap();
            assert_ne!(std::fs::read(asset.webm()).unwrap(), old);
            assert!(is_current(&asset.webm()));
        }
        Applied::Skipped(_) => assert_eq!(std::fs::read(asset.webm()).unwrap(), old),
    }
    assert!(asset.leftovers().is_empty());
}
