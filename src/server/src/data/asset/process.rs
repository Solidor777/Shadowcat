//! Image processing for the asset pipeline: WebP conversion of a staged
//! upload (original retained beside it when configured) plus thumb/preview
//! derivatives. Every function here is BLOCKING (`image` decode/encode is
//! CPU-bound) — callers run it under `tokio::task::spawn_blocking`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::AssetMeta;
use image::{AnimationDecoder, DynamicImage, ImageReader};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

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
/// File-name suffix of the retained original beside the canonical.
const ORIGINAL_SUFFIX: &str = ".orig";

/// A derivative size class.
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
pub fn derivative_path(canonical: &Path, variant: Variant) -> PathBuf {
    with_suffix(canonical, variant.suffix())
}

/// Path of the retained original beside `canonical`.
pub fn original_path(canonical: &Path) -> PathBuf {
    with_suffix(canonical, ORIGINAL_SUFFIX)
}

/// Every artifact that can sit beside a canonical: the retained original and
/// the two derivatives. The single statement of the sibling set — commit,
/// replace, delete and export all iterate this rather than re-spelling it.
pub fn sibling_paths(canonical: &Path) -> [PathBuf; 3] {
    [
        original_path(canonical),
        derivative_path(canonical, Variant::Thumb),
        derivative_path(canonical, Variant::Preview),
    ]
}

/// What `process_staged` decided about one upload.
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
pub fn write_derivatives(canonical: &Path) -> io::Result<()> {
    let img = ImageReader::open(canonical)?
        .with_guessed_format()?
        .decode()
        .map_err(io::Error::other)?;
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
        "image/gif" => image::codecs::gif::GifDecoder::new(reader)
            .map(|d| d.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        "image/webp" => image::codecs::webp::WebPDecoder::new(reader)
            .map(|d| d.has_animation())
            .unwrap_or(false),
        _ => false,
    }
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
    let img = match ImageReader::open(staged)?.with_guessed_format()?.decode() {
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
        return Ok(pass_through(
            original_content_type,
            original_byte_size,
            Some((&img, transparent)),
            true,
            Some("animated".into()),
        ));
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
        },
        converted: true,
    })
}

#[cfg(test)]
mod tests;
