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

/// A GIF whose header declares a 65535×65535 canvas, followed by nothing
/// worth decoding: a few dozen bytes on disk, ~17 GiB if the canvas were
/// allocated.
fn gif_with_huge_canvas() -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]); // width, height (u16 LE)
    v.extend_from_slice(&[0x00, 0x00, 0x00]); // flags, bg index, aspect
    v.extend_from_slice(&[0x2C, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]); // image descriptor
    v.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00, 0x3B]); // minimal LZW + trailer
    v
}

#[test]
fn gif_with_huge_declared_canvas_is_refused_before_any_canvas_allocates() {
    let dir = tempfile::tempdir().unwrap();
    let input = gif_with_huge_canvas();
    let staged = stage(dir.path(), &input);
    // If either the animation probe or the decode allocated the declared
    // canvas this test would exhaust memory instead of returning.
    let p = process_staged(&staged, "image/gif", input.len() as i64, true).unwrap();
    assert!(!p.converted);
    assert_eq!(p.content_type, "image/gif");
    assert!(!p.meta.animated);
    assert_eq!(p.meta.width, None);
    assert!(p
        .meta
        .conversion_note
        .as_deref()
        .is_some_and(|n| n.starts_with("decode failed")));
    assert!(!derivative_path(&staged, Variant::Thumb).exists());
    assert_eq!(std::fs::read(&staged).unwrap(), input);
}

#[test]
fn png_over_the_axis_bound_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    // A 1×(MAX+1) PNG is cheap to encode (one column) but declares an axis
    // over the bound.
    let img = RgbaImage::from_pixel(1, MAX_DECODE_AXIS_PX + 1, image::Rgba([1, 2, 3, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    let input = out.into_inner();
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/png", input.len() as i64, true).unwrap();
    assert!(!p.converted);
    assert_eq!(p.meta.width, None);
    assert!(p
        .meta
        .conversion_note
        .as_deref()
        .is_some_and(|n| n.starts_with("decode failed")));
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
fn sibling_suffixes_are_the_variant_suffixes_plus_orig_and_sheet() {
    let c = Path::new("c");
    assert_eq!(
        sibling_paths(c).to_vec(),
        vec![
            original_path(c),
            derivative_path(c, Variant::Thumb),
            derivative_path(c, Variant::Preview),
            with_suffix(c, ".sheet.webp"),
            with_suffix(c, ".sheet.json"),
        ]
    );
    assert_eq!(
        SIBLING_SUFFIXES,
        [
            ".orig",
            ".thumb.webp",
            ".preview.webp",
            ".sheet.webp",
            ".sheet.json"
        ]
    );
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

/// A 3-frame `w`×`h` GIF with distinct solid colors and delays 100/200/300 ms.
fn gif_frames_ms(w: u32, h: u32, delays_ms: &[u32]) -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame};
    let colors = [
        image::Rgba([255, 0, 0, 255]),
        image::Rgba([0, 255, 0, 255]),
        image::Rgba([0, 0, 255, 255]),
    ];
    let mut out = Cursor::new(Vec::new());
    {
        let mut enc = GifEncoder::new(&mut out);
        let frames = delays_ms.iter().enumerate().map(|(i, ms)| {
            Frame::from_parts(
                RgbaImage::from_pixel(w, h, colors[i % colors.len()]),
                0,
                0,
                Delay::from_numer_denom_ms(*ms, 1),
            )
        });
        enc.encode_frames(frames).unwrap();
    }
    out.into_inner()
}

#[test]
fn generate_grid_sheet_tiles_a_three_frame_gif_with_its_timings() {
    let dir = tempfile::tempdir().unwrap();
    let input = gif_frames_ms(4, 4, &[100, 200, 300]);
    let staged = stage(dir.path(), &input);

    let meta = generate_grid_sheet(&staged, "image/gif").unwrap();
    assert_eq!(meta.count, 3);
    assert_eq!(meta.rows, 2);
    assert_eq!(meta.cols, 2);
    assert_eq!(meta.frame_ms, vec![100, 200, 300]);
    assert_eq!((meta.width, meta.height), (4, 4));

    let sheet = with_suffix(&staged, ".sheet.webp");
    let json = with_suffix(&staged, ".sheet.json");
    assert!(sheet.exists());
    assert!(json.exists());
    let round: SheetMeta = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(round, meta);
}

#[test]
fn a_still_image_produces_no_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let input = png_rgba(16, 16);
    let staged = stage(dir.path(), &input);
    let p = process_staged(&staged, "image/png", input.len() as i64, false).unwrap();
    assert!(p.meta.sheet.is_none());
    assert!(!with_suffix(&staged, ".sheet.webp").exists());
    assert!(!with_suffix(&staged, ".sheet.json").exists());
}

#[test]
fn a_grid_over_the_max_axis_downscales_every_frame_uniformly() {
    let dir = tempfile::tempdir().unwrap();
    // 9 frames of 1500×1500: a 3×3 tiling would reach 4500px, over the cap.
    let input = gif_frames_ms(1500, 1500, &[100, 100, 100, 100, 100, 100, 100, 100, 100]);
    let staged = stage(dir.path(), &input);

    let meta = generate_grid_sheet(&staged, "image/gif").unwrap();
    assert_eq!((meta.rows, meta.cols, meta.count), (3, 3, 9));
    assert!(meta.width < 1500, "downscaled: {}", meta.width);
    assert!(meta.height < 1500, "downscaled: {}", meta.height);
    assert_eq!(
        meta.width, meta.height,
        "uniform downscale keeps square frames square"
    );
    // The written sheet's own dimensions are exactly the tiled downscaled frames.
    assert_eq!(
        webp_dims(&with_suffix(&staged, ".sheet.webp")),
        (meta.width * meta.cols, meta.height * meta.rows)
    );
}

#[test]
fn reprocessing_the_same_animation_regenerates_an_identical_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let input = gif_two_frames();
    let first = stage(dir.path(), &input);
    let p1 = process_staged(&first, "image/gif", input.len() as i64, true).unwrap();
    let second = stage(dir.path(), &input);
    let p2 = process_staged(&second, "image/gif", input.len() as i64, true).unwrap();
    let s1 = p1.meta.sheet.expect("first processing derives a sheet");
    let s2 = p2.meta.sheet.expect("reprocessing regenerates a sheet");
    assert_eq!(s1, s2);
}

#[test]
fn the_committed_animated_webp_fixture_decodes_through_the_webp_arm() {
    let bytes = include_bytes!("tests/fixtures/animated-4f-8x8.webp");
    let dir = tempfile::tempdir().unwrap();
    let staged = stage(dir.path(), bytes);

    let meta = generate_grid_sheet(&staged, "image/webp").unwrap();
    assert_eq!(meta.count, 4);
    assert_eq!(meta.rows, 2);
    assert_eq!(meta.cols, 2);
    assert_eq!(meta.frame_ms, vec![100, 100, 100, 100]);
    assert_eq!((meta.width, meta.height), (8, 8));

    let sheet = with_suffix(&staged, ".sheet.webp");
    let json = with_suffix(&staged, ".sheet.json");
    assert!(sheet.exists());
    assert!(json.exists());
    let round: SheetMeta = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(round, meta);

    // Frame index 2 tiles at (col = 2 % 2, row = 2 / 2) = (0, 1); the fixture
    // paints frame i's rows 2i/2i+1 red/blue, so that tile's rows 4/5 are
    // red/blue. The VP8L decode of the fixture's pure red/blue lands one
    // channel-step off (a fixed-point color-transform inversion artifact of
    // the decoder), so the assertion pins the decoded values.
    let img = ImageReader::open(&sheet)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .to_rgba8();
    let (col, row) = (2 % meta.cols, 2 / meta.cols);
    let tile = image::imageops::crop_imm(
        &img,
        col * meta.width,
        row * meta.height,
        meta.width,
        meta.height,
    )
    .to_image();
    assert_eq!(tile.get_pixel(0, 4).0, [254, 0, 0, 255]);
    assert_eq!(tile.get_pixel(0, 5).0, [0, 0, 254, 255]);
}

#[test]
fn generate_grid_sheet_composites_a_sub_rect_frame_at_its_offset() {
    // Committed fixture (123 bytes): an 8×8 logical screen; frame 0 solid red, frame 1 a 4×4
    // blue sub-image at left=2, top=2 (written with Pillow — the vendored `image` crate's
    // `GifEncoder` drops `Frame::left`/`top` when encoding, so an in-test generated fixture
    // can never carry a real offset, exactly why the animated-WebP fixture is committed too).
    // The decoder must composite the sub-rect at its offset, not smear it from (0,0).
    let bytes = include_bytes!("tests/fixtures/offset-4x4-at-2x2.gif");
    let dir = tempfile::tempdir().unwrap();
    let staged = stage(dir.path(), bytes);

    let meta = generate_grid_sheet(&staged, "image/gif").unwrap();
    assert_eq!((meta.count, meta.rows, meta.cols), (2, 1, 2));
    assert_eq!((meta.width, meta.height), (8, 8));

    let sheet = with_suffix(&staged, ".sheet.webp");
    let img = ImageReader::open(&sheet)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .to_rgba8();
    // Tile 1 (the second frame) is the red canvas with the blue 4×4 block at (2,2) — a
    // decoder that yielded the raw sub-rect instead would paint blue at (0,0). Channel
    // thresholds, not exact values: the VP8L round-trip lands pure red/blue one step off
    // (the same decoder artifact the fixture test above pins).
    let tile1 = image::imageops::crop_imm(&img, meta.width, 0, meta.width, meta.height).to_image();
    let blue_at =
        |x: u32, y: u32| tile1.get_pixel(x, y).0[2] >= 250 && tile1.get_pixel(x, y).0[0] <= 10;
    let red_at =
        |x: u32, y: u32| tile1.get_pixel(x, y).0[0] >= 250 && tile1.get_pixel(x, y).0[2] <= 10;
    assert!(
        blue_at(3, 3),
        "offset block center: {:?}",
        tile1.get_pixel(3, 3)
    );
    assert!(blue_at(2, 2), "block corner: {:?}", tile1.get_pixel(2, 2));
    assert!(
        red_at(0, 0),
        "outside the block: {:?}",
        tile1.get_pixel(0, 0)
    );
    assert!(red_at(6, 6), "past the block: {:?}", tile1.get_pixel(6, 6));
}

#[test]
fn generate_grid_sheet_clamps_a_zero_delay_frame_to_the_floor() {
    let dir = tempfile::tempdir().unwrap();
    let staged = stage(dir.path(), &gif_frames_ms(4, 4, &[100, 0, 300]));

    let meta = generate_grid_sheet(&staged, "image/gif").unwrap();
    assert_eq!(meta.frame_ms, vec![100, 100, 300]);
}

#[test]
fn generate_grid_sheet_never_exceeds_the_max_axis_after_downscale() {
    let dir = tempfile::tempdir().unwrap();
    // 257 frames of 250×250 tile 17 wide: the naive round() lands each frame at 241 px and
    // the sheet at 17×241 = 4097, one pixel over the cap — floor() must hold it under.
    let input = gif_frames_ms(250, 250, &[100].repeat(257));
    let staged = stage(dir.path(), &input);

    let meta = generate_grid_sheet(&staged, "image/gif").unwrap();
    assert_eq!(meta.cols, 17);
    assert!(
        meta.width * meta.cols <= 4096,
        "tiled width {} over the cap",
        meta.width * meta.cols
    );
    assert!(
        meta.height * meta.rows <= 4096,
        "tiled height {} over the cap",
        meta.height * meta.rows
    );
}
