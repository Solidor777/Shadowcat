//! Image processing for the asset pipeline: WebP conversion of a staged
//! upload (original retained beside it when configured) plus thumb/preview
//! derivatives. Every function here is BLOCKING (`image` decode/encode is
//! CPU-bound) — callers run it under `tokio::task::spawn_blocking`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::AssetMeta;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageReader};
use serde::{Deserialize, Serialize};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// Longest axis of the `thumb` derivative, in pixels.
pub const THUMB_PX: u32 = 128;
/// Longest axis of the `preview` derivative, in pixels.
pub const PREVIEW_PX: u32 = 512;
/// libwebp quality for a lossy canonical (JPEG-class sources).
pub const LOSSY_QUALITY: f32 = 85.0;
/// libwebp quality for lossy derivatives; lower than the canonical because a
/// derivative is a preview, never the served art.
const DERIVATIVE_QUALITY: f32 = 80.0;
/// MIME type of every converted canonical and every derivative.
pub const WEBP_CONTENT_TYPE: &str = "image/webp";
/// Largest axis any decode admits; a header declaring more is refused before
/// a pixel buffer exists (pass-through, `conversion_note`).
pub const MAX_DECODE_AXIS_PX: u32 = 16_384;
/// Largest pixel-buffer allocation any decode admits (256 MiB) — well under
/// the `image` crate's 512 MiB default, pinned here rather than inherited
/// because the crate documents its default as subject to change, and the
/// chat link-preview path reaches this decoder without elevated privilege.
pub const MAX_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

/// The bound every decoder and animation probe in this module runs under.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_AXIS_PX);
    limits.max_image_height = Some(MAX_DECODE_AXIS_PX);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    limits
}

/// File-name suffix of the retained original beside the canonical.
const ORIGINAL_SUFFIX: &str = ".orig";
/// File-name suffix of the server-derived grid-sheet image (animated sources only).
const SHEET_SUFFIX: &str = ".sheet.webp";
/// File-name suffix of the server-derived grid-sheet's timing/geometry sidecar.
const SHEET_JSON_SUFFIX: &str = ".sheet.json";
/// Longest axis (px) the grid sheet's FULL tiled image may reach; frames are downscaled
/// uniformly (never upscaled) when the near-square tiling would exceed it.
const SHEET_MAX_PX: u32 = 4096;

/// Every artifact that can sit beside a canonical: the retained original, the two
/// derivatives, and the two grid-sheet siblings (animated sources only). The single
/// statement of the sibling set — commit, replace, delete and export all iterate this
/// rather than re-spelling it. The world bundle accepts exactly this set under
/// `assets/<id><suffix>`.
pub const SIBLING_SUFFIXES: [&str; 5] = [
    ORIGINAL_SUFFIX,
    ".thumb.webp",
    ".preview.webp",
    SHEET_SUFFIX,
    SHEET_JSON_SUFFIX,
];

/// A derivative size class.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::{derivative_path, Variant};
/// use std::path::Path;
///
/// let canonical = Path::new("assets").join("uuid");
/// let thumb = derivative_path(&canonical, Variant::Thumb);
/// assert!(thumb.to_string_lossy().ends_with(".thumb.webp"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Grid tile (`THUMB_PX`).
    Thumb,
    /// Detail pane (`PREVIEW_PX`).
    Preview,
}

impl Variant {
    /// File-name suffix appended to the canonical path.
    fn suffix(self) -> &'static str {
        match self {
            Variant::Thumb => ".thumb.webp",
            Variant::Preview => ".preview.webp",
        }
    }

    /// Longest-axis bound in pixels.
    fn max_px(self) -> u32 {
        match self {
            Variant::Thumb => THUMB_PX,
            Variant::Preview => PREVIEW_PX,
        }
    }
}

/// `path` with `suffix` appended to its final component (`<uuid>` →
/// `<uuid>.thumb.webp`), keeping the directory. Built on the OS string, never
/// a separator literal, so it is the same on every platform.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Path of the `variant` derivative beside `canonical`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::{derivative_path, Variant};
/// use std::path::Path;
///
/// let canonical = Path::new("data").join("uuid");
/// let preview = derivative_path(&canonical, Variant::Preview);
/// assert_eq!(preview, Path::new("data").join("uuid.preview.webp"));
/// ```
pub fn derivative_path(canonical: &Path, variant: Variant) -> PathBuf {
    with_suffix(canonical, variant.suffix())
}

/// Path of the retained original beside `canonical`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::original_path;
/// use std::path::Path;
///
/// let canonical = Path::new("data").join("uuid");
/// assert_eq!(original_path(&canonical), Path::new("data").join("uuid.orig"));
/// ```
pub fn original_path(canonical: &Path) -> PathBuf {
    with_suffix(canonical, ORIGINAL_SUFFIX)
}

/// Every artifact that can sit beside a canonical: the retained original and
/// the two derivatives, plus the two grid-sheet siblings for an animated
/// source. The single statement of the sibling set — commit, replace, delete
/// and export all iterate this rather than re-spelling it.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::sibling_paths;
/// use std::path::Path;
///
/// let canonical = Path::new("data").join("uuid");
/// let siblings = sibling_paths(&canonical);
/// assert_eq!(siblings.len(), 5);
/// assert!(siblings[0].to_string_lossy().ends_with(".orig"));
/// ```
pub fn sibling_paths(canonical: &Path) -> [PathBuf; 5] {
    SIBLING_SUFFIXES.map(|suffix| with_suffix(canonical, suffix))
}

/// What `process_staged` decided about one upload.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::{process_staged, Processed};
/// use std::io::Cursor;
///
/// // A directory outside the repo tree — never written to source control.
/// let dir = tempfile::tempdir().unwrap();
/// let staged = dir.path().join("upload");
/// let mut png = Vec::new();
/// image::RgbaImage::new(2, 2)
///     .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
///     .unwrap();
/// std::fs::write(&staged, &png).unwrap();
/// // A real PNG decodes and re-encodes: `converted`/`content_type` come from the
/// // pipeline's own decision, not from a literal.
/// let processed: Processed =
///     process_staged(&staged, "image/png", png.len() as i64, false).unwrap();
/// assert!(processed.converted);
/// assert_eq!(processed.content_type, "image/webp");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Processed {
    /// MIME type of the canonical file now at the staged path.
    pub content_type: String,
    /// Size of the canonical file now at the staged path.
    pub byte_size: i64,
    /// Pipeline metadata to record on the row.
    pub meta: AssetMeta,
    /// Whether the canonical is a re-encode (`true`) or the arrived bytes (`false`).
    pub converted: bool,
}

/// How a decoded image is encoded to WebP.
#[derive(Debug, Clone, Copy)]
struct Encoding {
    /// `true` → `encode_lossless`, else `encode(quality)`.
    lossless: bool,
    /// Quality for the lossy branch.
    quality: f32,
}

/// Encode `img` as WebP bytes. The buffer is always RGBA8: libwebp needs a
/// packed 8-bit layout and `DynamicImage` may hold 16-bit or grayscale data.
fn encode_webp(img: &DynamicImage, enc: Encoding) -> io::Result<Vec<u8>> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let encoder = webp::Encoder::from_rgba(rgba.as_raw(), w, h);
    let mem = if enc.lossless {
        encoder.encode_lossless()
    } else {
        encoder.encode(enc.quality)
    };
    if mem.is_empty() {
        return Err(io::Error::other("libwebp produced no output"));
    }
    Ok(mem.to_vec())
}

/// Write `bytes` to `dest` through a sibling temp file + rename, so a reader
/// never observes a partially written derivative.
fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = with_suffix(dest, &format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Whether any pixel is not fully opaque — the `transparent` signal. A PNG
/// that merely CARRIES an all-opaque alpha channel is not transparent.
fn has_transparent_pixels(img: &DynamicImage) -> bool {
    if !img.color().has_alpha() {
        return false;
    }
    img.to_rgba8().pixels().any(|p| p[3] < u8::MAX)
}

/// `img` scaled to fit within `max_px` on its longest axis; never upscaled.
fn fit_within(img: &DynamicImage, max_px: u32) -> DynamicImage {
    if img.width() <= max_px && img.height() <= max_px {
        return img.clone();
    }
    // `thumbnail` keeps the aspect ratio inside the box (Triangle filter — a
    // preview does not warrant Lanczos cost).
    img.thumbnail(max_px, max_px)
}

/// Write both derivatives of `img` beside `canonical`. Lossless when the
/// source is transparent (alpha must survive), else lossy at
/// `DERIVATIVE_QUALITY`.
fn write_derivatives_of(img: &DynamicImage, canonical: &Path, transparent: bool) -> io::Result<()> {
    let enc = Encoding {
        lossless: transparent,
        quality: DERIVATIVE_QUALITY,
    };
    for variant in [Variant::Thumb, Variant::Preview] {
        let scaled = fit_within(img, variant.max_px());
        let bytes = encode_webp(&scaled, enc)?;
        write_atomic(&derivative_path(canonical, variant), &bytes)?;
    }
    Ok(())
}

/// Regenerate both derivatives from the canonical file (the on-demand path
/// for a missing derivative). Fails when the canonical does not decode.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::write_derivatives;
/// use std::path::Path;
///
/// // A canonical that isn't on disk fails to decode rather than panicking.
/// let err = write_derivatives(Path::new("no-such-canonical.webp")).unwrap_err();
/// assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
/// ```
pub fn write_derivatives(canonical: &Path) -> io::Result<()> {
    let mut reader = ImageReader::open(canonical)?.with_guessed_format()?;
    reader.limits(decode_limits());
    let img = reader.decode().map_err(io::Error::other)?;
    let transparent = has_transparent_pixels(&img);
    write_derivatives_of(&img, canonical, transparent)
}

/// Whether the file at `path` is a multi-frame animation for `content_type`.
/// Only GIF and WebP can animate; any decoder error reads as "not animated"
/// and is left for the full decode to report.
fn is_animated(path: &Path, content_type: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file);
    match content_type {
        // A raw codec starts with NO limits (`Limits::no_limits()`); the frame
        // iterator allocates a header-sized canvas before reading a pixel, so
        // the bound must be set here, on the probe, not only on `decode`. A
        // canvas over the bound reads as "not animated" and the later decode
        // refuses it for the same reason.
        "image/gif" => image::codecs::gif::GifDecoder::new(reader)
            .and_then(|mut d| d.set_limits(decode_limits()).map(|()| d))
            .map(|d| d.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        "image/webp" => image::codecs::webp::WebPDecoder::new(reader)
            .map(|d| d.has_animation())
            .unwrap_or(false),
        _ => false,
    }
}

/// Decode every frame of an animated GIF/WebP at `path`, honoring the same decode limits
/// every other decode in this module runs under. Returns an empty vec for a content type
/// this module cannot decode as an animation (the caller only reaches this after
/// `is_animated` already confirmed one of the two supported types) or on any decode error —
/// callers treat an empty result as "sheet generation produced nothing", never a hard
/// failure (an upload/reconvert is never rejected for a sheet-generation reason, mirroring
/// `process_staged`'s own pass-through-on-failure convention).
fn decode_animation_frames(path: &Path, content_type: &str) -> Vec<Frame> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let frames = match content_type {
        "image/gif" => image::codecs::gif::GifDecoder::new(reader)
            .and_then(|mut d| d.set_limits(decode_limits()).map(|()| d))
            .map(|d| d.into_frames().collect_frames()),
        "image/webp" => image::codecs::webp::WebPDecoder::new(reader)
            .and_then(|mut d| d.set_limits(decode_limits()).map(|()| d))
            .map(|d| d.into_frames().collect_frames()),
        _ => return Vec::new(),
    };
    match frames {
        Ok(Ok(frames)) => frames,
        _ => Vec::new(),
    }
}

/// Server-derived grid-sheet geometry + per-frame timing, recorded on `AssetMeta.sheet` and
/// mirrored verbatim into the `.sheet.json` sidecar. `width`/`height` are the PER-FRAME pixel
/// dimensions after any downscale (never the full tiled sheet's dimensions) — the render
/// client already derives per-frame size by dividing the loaded sheet texture by
/// `cols`/`rows`, so this pair exists for tooling/documentation, not client consumption.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::SheetMeta;
///
/// let meta = SheetMeta { rows: 2, cols: 2, count: 3, frame_ms: vec![100, 100, 100], width: 8, height: 8 };
/// assert_eq!(meta.count, 3);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct SheetMeta {
    /// Grid row count.
    pub rows: u32,
    /// Grid column count.
    pub cols: u32,
    /// Frame count (`<= rows*cols`; the tiling is near-square, so the last row/col may be
    /// partially empty).
    pub count: u32,
    /// Per-frame display duration in milliseconds, in playback order.
    pub frame_ms: Vec<u32>,
    /// Per-frame pixel width, after any uniform downscale.
    pub width: u32,
    /// Per-frame pixel height, after any uniform downscale.
    pub height: u32,
}

/// Tile every decoded frame of an animated GIF/WebP into a near-square `rows`×`cols` grid
/// (`cols = ceil(sqrt(count))`, `rows = ceil(count/cols)`), downscaling every frame UNIFORMLY
/// (never upscaling) when the full tiled sheet's longest side would exceed `SHEET_MAX_PX`,
/// and writes `<canonical>.sheet.webp` (always LOSSLESS — the sheet's transparency must
/// survive exactly, unlike the lossy-when-opaque derivative thumbnails) + `<canonical>.sheet.json`
/// (the same `SheetMeta` serialized, so a re-import can restore `AssetMeta.sheet` from the
/// sibling file alone, without re-decoding the source animation). Returns `None` (no files
/// written) for fewer than 2 decoded frames — nothing to tile. A write failure partway
/// through is a best-effort no-op: the caller (`process_staged`) never fails an upload for a
/// sheet-generation reason, matching `write_derivatives_of`'s own best-effort convention.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::generate_grid_sheet;
/// use std::path::Path;
///
/// // No frames decode from a nonexistent file; nothing is written.
/// assert!(generate_grid_sheet(Path::new("no-such-file"), "image/gif").is_none());
/// ```
pub fn generate_grid_sheet(canonical: &Path, content_type: &str) -> Option<SheetMeta> {
    let frames = decode_animation_frames(canonical, content_type);
    if frames.len() < 2 {
        return None;
    }
    let count = frames.len() as u32;
    let cols = (f64::from(count)).sqrt().ceil() as u32;
    let rows = count.div_ceil(cols);
    let frame_ms: Vec<u32> = frames
        .iter()
        .map(|f| {
            let (numer, denom) = f.delay().numer_denom_ms();
            numer.checked_div(denom).unwrap_or(0)
        })
        .collect();
    let (fw0, fh0) = frames[0].buffer().dimensions();
    if fw0 == 0 || fh0 == 0 {
        return None;
    }
    let sheet_w = fw0.saturating_mul(cols);
    let sheet_h = fh0.saturating_mul(rows);
    let longest = sheet_w.max(sheet_h);
    let scale = if longest > SHEET_MAX_PX {
        f64::from(SHEET_MAX_PX) / f64::from(longest)
    } else {
        1.0
    };
    let fw = ((f64::from(fw0) * scale).round() as u32).max(1);
    let fh = ((f64::from(fh0) * scale).round() as u32).max(1);
    let mut canvas = image::RgbaImage::new(fw * cols, fh * rows);
    for (i, frame) in frames.iter().enumerate() {
        let img = DynamicImage::ImageRgba8(frame.buffer().clone());
        let scaled = if scale < 1.0 {
            img.resize_exact(fw, fh, FilterType::Triangle)
        } else {
            img
        };
        let i = i as u32;
        let col = i % cols;
        let row = i / cols;
        image::imageops::overlay(
            &mut canvas,
            &scaled.to_rgba8(),
            i64::from(col * fw),
            i64::from(row * fh),
        );
    }
    let sheet_bytes = encode_webp(
        &DynamicImage::ImageRgba8(canvas),
        Encoding {
            lossless: true,
            quality: LOSSY_QUALITY,
        },
    )
    .ok()?;
    write_atomic(&with_suffix(canonical, SHEET_SUFFIX), &sheet_bytes).ok()?;
    let meta = SheetMeta {
        rows,
        cols,
        count,
        frame_ms,
        width: fw,
        height: fh,
    };
    let json_bytes = serde_json::to_vec(&meta).ok()?;
    write_atomic(&with_suffix(canonical, SHEET_JSON_SUFFIX), &json_bytes).ok()?;
    Some(meta)
}

/// Whether a converted canonical must be lossless: the source is transparent
/// (alpha survives only losslessly) or belongs to a lossless family, where a
/// lossy re-encode would degrade pixel art / line art.
fn wants_lossless(original_content_type: &str, transparent: bool) -> bool {
    transparent
        || matches!(
            original_content_type,
            "image/png" | "image/gif" | "image/bmp" | "image/tiff"
        )
}

/// The pass-through outcome: the arrived bytes stay the canonical, with the
/// metadata `decoded` (if any) provides and `note` as the reason.
fn pass_through(
    original_content_type: &str,
    original_byte_size: i64,
    decoded: Option<(&DynamicImage, bool)>,
    animated: bool,
    note: Option<String>,
) -> Processed {
    let mut meta = AssetMeta::unprocessed(original_content_type, original_byte_size);
    if let Some((img, transparent)) = decoded {
        meta.width = Some(img.width());
        meta.height = Some(img.height());
        meta.has_alpha = transparent;
    }
    meta.animated = animated;
    meta.conversion_note = note;
    Processed {
        content_type: original_content_type.to_string(),
        byte_size: original_byte_size,
        meta,
        converted: false,
    }
}

/// Process the upload staged at `staged` (BLOCKING). On return the canonical
/// bytes are at `staged` — rewritten in place when converted — the original
/// (when converted AND `retain_originals`) at `original_path(staged)`, and
/// both derivatives at `derivative_path(staged, _)` whenever the source
/// decoded at all.
///
/// Pass-through (canonical = the arrived bytes, `converted: false`) for: a
/// non-`image/*` type, SVG, an animation (never re-encoded), a static WebP
/// (nothing to gain), and any decode failure. A conversion failure after a
/// successful decode also falls back to pass-through with the reason in
/// `conversion_note` — an upload is never rejected for conversion reasons.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::process_staged;
/// use std::path::Path;
///
/// // A missing staged file fails to open rather than panicking.
/// let err = process_staged(Path::new("no-such-staged-upload"), "image/png", 0, true)
///     .unwrap_err();
/// assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
/// ```
pub fn process_staged(
    staged: &Path,
    original_content_type: &str,
    original_byte_size: i64,
    retain_originals: bool,
) -> io::Result<Processed> {
    if !original_content_type.starts_with("image/") {
        return Ok(pass_through(
            original_content_type,
            original_byte_size,
            None,
            false,
            Some("not an image".into()),
        ));
    }
    if original_content_type == "image/svg+xml" {
        return Ok(pass_through(
            original_content_type,
            original_byte_size,
            None,
            false,
            Some("svg".into()),
        ));
    }
    let animated = is_animated(staged, original_content_type);
    // `decode` yields the first frame of an animation — enough for dimensions
    // and derivatives.
    let mut reader = ImageReader::open(staged)?.with_guessed_format()?;
    reader.limits(decode_limits());
    let img = match reader.decode() {
        Ok(img) => img,
        Err(e) => {
            return Ok(pass_through(
                original_content_type,
                original_byte_size,
                None,
                false,
                Some(format!("decode failed: {e}")),
            ));
        }
    };
    let transparent = has_transparent_pixels(&img);
    // Derivatives come from whatever decoded, converted or not; a derivative
    // failure is not a reason to reject the upload (regenerated on demand).
    if let Err(e) = write_derivatives_of(&img, staged, transparent) {
        tracing::warn!(?e, path = %staged.display(), "derivative write failed");
    }
    if animated {
        let sheet = generate_grid_sheet(staged, original_content_type);
        let mut processed = pass_through(
            original_content_type,
            original_byte_size,
            Some((&img, transparent)),
            true,
            Some("animated".into()),
        );
        processed.meta.sheet = sheet;
        return Ok(processed);
    }
    if original_content_type == WEBP_CONTENT_TYPE {
        return Ok(pass_through(
            original_content_type,
            original_byte_size,
            Some((&img, transparent)),
            false,
            None,
        ));
    }

    let enc = Encoding {
        lossless: wants_lossless(original_content_type, transparent),
        quality: LOSSY_QUALITY,
    };
    let bytes = match encode_webp(&img, enc) {
        Ok(b) => b,
        Err(e) => {
            return Ok(pass_through(
                original_content_type,
                original_byte_size,
                Some((&img, transparent)),
                false,
                Some(format!("encode failed: {e}")),
            ));
        }
    };
    // Swap order: the converted bytes land in a sibling temp first, then the
    // arrived bytes move aside (or go), then the temp takes the canonical
    // name — at no point is the canonical path missing AND the original gone.
    let conv_tmp = with_suffix(staged, ".conv.tmp");
    std::fs::write(&conv_tmp, &bytes)?;
    if retain_originals {
        std::fs::rename(staged, original_path(staged))?;
    } else {
        std::fs::remove_file(staged)?;
    }
    std::fs::rename(&conv_tmp, staged)?;

    Ok(Processed {
        content_type: WEBP_CONTENT_TYPE.to_string(),
        byte_size: bytes.len() as i64,
        meta: AssetMeta {
            width: Some(img.width()),
            height: Some(img.height()),
            has_alpha: transparent,
            animated: false,
            original_content_type: original_content_type.to_string(),
            original_byte_size,
            original_retained: retain_originals,
            conversion_note: None,
            sheet: None,
        },
        converted: true,
    })
}

#[cfg(test)]
mod tests;
