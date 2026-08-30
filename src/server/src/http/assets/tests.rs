use super::*;
use uuid::Uuid;

#[test]
fn detects_supported_image_signatures_and_rejects_others() {
    assert_eq!(
        detect_image_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        Some("image/png")
    );
    assert_eq!(
        detect_image_type(&[0xFF, 0xD8, 0xFF, 0x00]),
        Some("image/jpeg")
    );
    assert_eq!(detect_image_type(b"GIF89a..."), Some("image/gif"));
    assert_eq!(
        detect_image_type(b"RIFF\0\0\0\0WEBPxxxx"),
        Some("image/webp")
    );
    assert_eq!(detect_image_type(b"%PDF-1.7"), None);
    assert_eq!(detect_image_type(b"<svg xmlns="), Some("image/svg+xml"));
    assert_eq!(
        detect_image_type(b"\xEF\xBB\xBF  <?xml versio"),
        Some("image/svg+xml")
    );
    assert_eq!(detect_image_type(b"<html><body>"), None);
    assert_eq!(detect_image_type(b"BM\x36\x00\x00\x00"), Some("image/bmp"));
    assert_eq!(detect_image_type(b"II*\x00\x08\x00"), Some("image/tiff"));
    assert_eq!(detect_image_type(b"MM\x00*\x00\x00"), Some("image/tiff"));
    assert_eq!(detect_image_type(&[0x89]), None); // too short to decide
}

#[test]
fn rate_limiter_trips_after_per_min_then_window_slides() {
    let rl = UploadRateLimiter::new();
    let u = Uuid::from_u128(1);
    assert!(rl.check(u, 1_000, 2));
    assert!(rl.check(u, 1_500, 2));
    assert!(!rl.check(u, 1_800, 2)); // 3rd within the window → rejected
                                     // 61s later the earlier hits have aged out.
    assert!(rl.check(u, 62_001, 2));
}
