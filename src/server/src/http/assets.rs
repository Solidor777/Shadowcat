use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Detect a supported image content-type from leading bytes, else `None`.
/// The bytes are the validation boundary — the client-declared content-type is
/// never trusted. Needs ≥12 bytes to rule on WebP. Source: file-format magic
/// numbers (PNG/JFIF/GIF/RIFF specs).
pub fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Per-user sliding-window upload limiter (trailing 60s). In-memory; resets on
/// restart, which is acceptable for an abuse backstop.
pub struct UploadRateLimiter {
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl UploadRateLimiter {
    pub fn new() -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Record an upload at `now_ms` and report whether it is within `per_min`.
    /// Prunes entries older than the 60s window first.
    pub fn check(&self, user: Uuid, now_ms: i64, per_min: u32) -> bool {
        let mut map = self.hits.lock().expect("rate-limiter mutex poisoned");
        let v = map.entry(user).or_default();
        let cutoff = now_ms - 60_000;
        v.retain(|&t| t > cutoff);
        if v.len() as u32 >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

impl Default for UploadRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn detects_supported_image_signatures_and_rejects_others() {
        assert_eq!(
            detect_image_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some("image/png")
        );
        assert_eq!(detect_image_type(&[0xFF, 0xD8, 0xFF, 0x00]), Some("image/jpeg"));
        assert_eq!(detect_image_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(detect_image_type(b"RIFF\0\0\0\0WEBPxxxx"), Some("image/webp"));
        assert_eq!(detect_image_type(b"%PDF-1.7"), None);
        assert_eq!(detect_image_type(b"<svg"), None); // SVG excluded in M8
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
}
