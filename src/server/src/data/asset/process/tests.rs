use super::*;
use image::{ImageFormat, RgbImage, RgbaImage};
use std::io::Cursor;

/// A `w`×`h` RGBA PNG with one fully transparent pixel at (0, 0).
fn png_rgba(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| {
        if x == 0 && y == 0 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([200, 30, 30, 255])
        }
    });
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A `w`×`h` opaque JPEG with a smooth gradient (compresses predictably).
fn jpeg_rgb(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    });
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

/// A two-frame 8×8 GIF.
fn gif_two_frames() -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let mut out = Cursor::new(Vec::new());
    {
        let mut enc = GifEncoder::new(&mut out);
        let frames = [image::Rgba([255, 0, 0, 255]), image::Rgba([0, 0, 255, 255])]
            .into_iter()
            .map(|px| {
                Frame::from_parts(
                    RgbaImage::from_pixel(8, 8, px),
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                )
            });
        enc.encode_frames(frames).unwrap();
    }
    out.into_inner()
}

/// Stage `bytes` as `<dir>/<uuid>` and return the path.
fn stage(dir: &Path, bytes: &[u8]) -> PathBuf {
    let p = dir.join(uuid::Uuid::new_v4().to_string());
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Decode a WebP derivative and return `(width, height)`; panics if not WebP.
fn webp_dims(path: &Path) -> (u32, u32) {
    let reader = ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap();
    assert_eq!(
        reader.format(),
        Some(ImageFormat::WebP),
        "{}",
        path.display()
    );
    let img = reader.decode().unwrap();
    (img.width(), img.height())
}

#[test]
fn png_with_alpha_converts_lossless_and_retains_original() {
    let dir = tempfile::tempdir().unwrap();
    let input = png_rgba(300, 200);
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/png", input.len() as i64, true).unwrap();

    assert_eq!(p.content_type, "image/webp");
    assert!(p.converted);
    assert!(p.meta.has_alpha);
    assert!(!p.meta.animated);
    assert_eq!((p.meta.width, p.meta.height), (Some(300), Some(200)));
    assert_eq!(p.meta.original_content_type, "image/png");
    assert_eq!(p.meta.original_byte_size, input.len() as i64);
    assert!(p.meta.original_retained);
    assert_eq!(p.meta.conversion_note, None);
    assert_eq!(
        p.byte_size,
        std::fs::metadata(&staged).unwrap().len() as i64
    );

    // The original is preserved byte-for-byte; the canonical is now WebP and
    // still transparent at (0, 0) — i.e. losslessly encoded with alpha.
    assert_eq!(std::fs::read(original_path(&staged)).unwrap(), input);
    let canonical = ImageReader::open(&staged)
        .unwrap()
        .with_guessed_format()
        .unwrap();
    assert_eq!(canonical.format(), Some(ImageFormat::WebP));
    let rgba = canonical.decode().unwrap().to_rgba8();
    assert_eq!(rgba.get_pixel(0, 0)[3], 0);
    assert_eq!(rgba.get_pixel(5, 5).0, [200, 30, 30, 255]);

    // Derivatives fit their boxes and keep the 3:2 aspect ratio.
    assert_eq!(
        webp_dims(&derivative_path(&staged, Variant::Thumb)),
        (128, 85)
    );
    assert_eq!(
        webp_dims(&derivative_path(&staged, Variant::Preview)),
        (300, 200)
    );
}

#[test]
fn jpeg_converts_lossy_without_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let input = jpeg_rgb(600, 400);
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/jpeg", input.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "image/webp");
    assert!(p.converted);
    assert!(!p.meta.has_alpha);
    assert_eq!((p.meta.width, p.meta.height), (Some(600), Some(400)));
    assert!(original_path(&staged).exists());
    assert_eq!(
        webp_dims(&derivative_path(&staged, Variant::Preview)),
        (512, 341)
    );
}

#[test]
fn retain_false_writes_no_orig() {
    let dir = tempfile::tempdir().unwrap();
    let input = png_rgba(16, 16);
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/png", input.len() as i64, false).unwrap();
    assert!(p.converted);
    assert!(!p.meta.original_retained);
    assert!(!original_path(&staged).exists());
    assert!(staged.exists());
    // Square source → square derivatives; a 16px source is never upscaled.
    assert_eq!(
        webp_dims(&derivative_path(&staged, Variant::Thumb)),
        (16, 16)
    );
}

#[test]
fn animated_gif_is_passthrough_with_note() {
    let dir = tempfile::tempdir().unwrap();
    let input = gif_two_frames();
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/gif", input.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "image/gif");
    assert!(!p.converted);
    assert!(p.meta.animated);
    assert_eq!(p.meta.conversion_note.as_deref(), Some("animated"));
    assert!(!p.meta.original_retained);
    assert_eq!((p.meta.width, p.meta.height), (Some(8), Some(8)));
    // Bytes untouched, no .orig, derivatives from frame 0.
    assert_eq!(std::fs::read(&staged).unwrap(), input);
    assert!(!original_path(&staged).exists());
    assert_eq!(webp_dims(&derivative_path(&staged, Variant::Thumb)), (8, 8));
}

#[test]
fn static_webp_is_passthrough_without_note() {
    let dir = tempfile::tempdir().unwrap();
    // Produce a real static WebP by converting a PNG first.
    let png = png_rgba(20, 10);
    let first = stage(dir.path(), &png);
    process_staged(&first, "image/png", png.len() as i64, false).unwrap();
    let webp_bytes = std::fs::read(&first).unwrap();

    let staged = stage(dir.path(), &webp_bytes);
    let p = process_staged(&staged, "image/webp", webp_bytes.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "image/webp");
    assert!(!p.converted);
    assert_eq!(p.meta.conversion_note, None);
    assert!(p.meta.has_alpha);
    assert!(!original_path(&staged).exists());
    assert_eq!(std::fs::read(&staged).unwrap(), webp_bytes);
}

#[test]
fn svg_and_undecodable_and_non_image_are_passthrough() {
    let dir = tempfile::tempdir().unwrap();

    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    let staged = stage(dir.path(), svg);
    let p = process_staged(&staged, "image/svg+xml", svg.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "image/svg+xml");
    assert!(!p.converted);
    assert_eq!(p.meta.conversion_note.as_deref(), Some("svg"));
    assert_eq!(p.meta.width, None);
    assert!(!derivative_path(&staged, Variant::Thumb).exists());

    let garbage = b"\x89PNG\r\n\x1a\nthis is not a png";
    let staged = stage(dir.path(), garbage);
    let p = process_staged(&staged, "image/png", garbage.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "image/png");
    assert!(!p.converted);
    assert!(p
        .meta
        .conversion_note
        .as_deref()
        .is_some_and(|n| n.starts_with("decode failed")));
    assert_eq!(p.meta.width, None);
    assert_eq!(std::fs::read(&staged).unwrap(), garbage);

    let pdf = b"%PDF-1.7";
    let staged = stage(dir.path(), pdf);
    let p = process_staged(&staged, "application/pdf", pdf.len() as i64, true).unwrap();
    assert_eq!(p.content_type, "application/pdf");
    assert_eq!(p.meta.conversion_note.as_deref(), Some("not an image"));
    assert!(!p.meta.original_retained);
}

#[test]
fn write_derivatives_regenerates_from_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let input = jpeg_rgb(1024, 256);
    let staged = stage(dir.path(), &input);
    process_staged(&staged, "image/jpeg", input.len() as i64, false).unwrap();
    let thumb = derivative_path(&staged, Variant::Thumb);
    let preview = derivative_path(&staged, Variant::Preview);
    std::fs::remove_file(&thumb).unwrap();
    std::fs::remove_file(&preview).unwrap();

    write_derivatives(&staged).unwrap();
    assert_eq!(webp_dims(&thumb), (128, 32));
    assert_eq!(webp_dims(&preview), (512, 128));

    // A canonical that does not decode is an error, not a silent no-op.
    let bad = stage(dir.path(), b"not an image at all");
    assert!(write_derivatives(&bad).is_err());
}

#[test]
fn suffix_paths_keep_the_directory_and_append_to_the_file_name() {
    let canonical = Path::new("worlds").join("abc");
    assert_eq!(
        derivative_path(&canonical, Variant::Thumb),
        Path::new("worlds").join("abc.thumb.webp")
    );
    assert_eq!(
        derivative_path(&canonical, Variant::Preview),
        Path::new("worlds").join("abc.preview.webp")
    );
    assert_eq!(
        original_path(&canonical),
        Path::new("worlds").join("abc.orig")
    );
}
