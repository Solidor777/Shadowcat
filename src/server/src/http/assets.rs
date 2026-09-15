#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod mutate;
pub mod query;
pub mod uploads;

use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Detect a supported image content-type from leading bytes, else `None`.
/// The bytes are the validation boundary — the client-declared content-type is
/// never trusted. Needs ≥12 bytes to rule on WebP. Source: file-format magic
/// numbers (PNG/JFIF/GIF/RIFF/BMP/TIFF specs); SVG is text, recognized by an
/// XML prolog or `<svg` root after an optional BOM/whitespace — every type
/// `data::asset::process` has a branch for is sniffable here, so an honest
/// declaration of one of them never collapses to octet-stream.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::detect_image_type;
///
/// let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// assert_eq!(detect_image_type(&png), Some("image/png"));
/// assert_eq!(detect_image_type(b"not an image"), None);
/// ```
pub fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"BM") && bytes.len() >= 6 {
        return Some("image/bmp");
    }
    if bytes.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || bytes.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        return Some("image/tiff");
    }
    let text_start = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(bytes)
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)[i..]);
    if let Some(t) = text_start {
        if t.starts_with(b"<?xml") || t.starts_with(b"<svg") {
            return Some("image/svg+xml");
        }
    }
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

/// Sniff an AUDIO container from the leading bytes — the upload pipeline's "the bytes decide"
/// counterpart to `detect_image_type`, so a mislabeled upload (`application/octet-stream` on
/// a real WAV) still reaches the transcode arm. Only audio-unambiguous magics are claimed
/// (RIFF/WAVE, FLAC, MP3, Ogg); EBML/Matroska is deliberately NOT sniffed here — a WebM file
/// may be video, and its declared label stands (the transcode probe would pass it through
/// harmlessly either way). The returned label is a candidate for
/// `process_staged`'s audio arm; `process::audio`'s own symphonia probe remains the real
/// container decision.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::detect_audio_type;
///
/// assert_eq!(detect_audio_type(b"RIFF\x00\x00\x00\x00WAVE"), Some("audio/wav"));
/// assert_eq!(detect_audio_type(b"fLaC\x00"), Some("audio/flac"));
/// assert_eq!(detect_audio_type(b"ID3\x04"), Some("audio/mpeg"));
/// assert_eq!(detect_audio_type(&[0xFF, 0xFB, 0x90, 0x00]), Some("audio/mpeg"));
/// assert_eq!(detect_audio_type(b"OggS\x00"), Some("audio/ogg"));
/// assert_eq!(detect_audio_type(b"not audio"), None);
/// ```
pub fn detect_audio_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    if bytes.starts_with(b"fLaC") {
        return Some("audio/flac");
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        return Some("audio/mpeg");
    }
    if bytes.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    None
}

/// Per-user sliding-window upload limiter (trailing 60s). In-memory; resets on
/// restart, which is acceptable for an abuse backstop.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::UploadRateLimiter;
/// use uuid::Uuid;
///
/// let limiter = UploadRateLimiter::new();
/// let user = Uuid::new_v4();
/// assert!(limiter.check(user, 0, 1)); // first upload within budget
/// assert!(!limiter.check(user, 0, 1)); // second within the same minute is refused
/// ```
pub struct UploadRateLimiter {
    /// Per-user hit timestamps within the trailing window.
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl UploadRateLimiter {
    /// An empty limiter (one per `AppState`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::http::assets::UploadRateLimiter;
    /// use uuid::Uuid;
    ///
    /// let limiter = UploadRateLimiter::new();
    /// assert!(limiter.check(Uuid::new_v4(), 0, 5)); // fresh limiter starts empty
    /// ```
    pub fn new() -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Record an upload at `now_ms` and report whether it is within `per_min`.
    /// Prunes entries older than the 60s window first.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::http::assets::UploadRateLimiter;
    /// use uuid::Uuid;
    ///
    /// let limiter = UploadRateLimiter::new();
    /// let user = Uuid::new_v4();
    /// assert!(limiter.check(user, 1_000, 2));
    /// assert!(limiter.check(user, 1_500, 2));
    /// assert!(!limiter.check(user, 1_800, 2)); // third within the window is refused
    /// ```
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

    /// Return a hit recorded by `check` (matched by `now_ms`) to the user's
    /// budget — called when the gated upload subsequently fails, so a rejected
    /// request (bad bytes, over-cap, I/O error) does not consume quota. The
    /// `check`-before-stream order still bounds in-flight concurrency.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::http::assets::UploadRateLimiter;
    /// use uuid::Uuid;
    ///
    /// let limiter = UploadRateLimiter::new();
    /// let user = Uuid::new_v4();
    /// assert!(limiter.check(user, 0, 1)); // consumes the only slot
    /// limiter.refund(user, 0); // a failed upload gives it back
    /// assert!(limiter.check(user, 0, 1));
    /// ```
    pub fn refund(&self, user: Uuid, now_ms: i64) {
        let mut map = self.hits.lock().expect("rate-limiter mutex poisoned");
        if let Some(v) = map.get_mut(&user) {
            if let Some(pos) = v.iter().rposition(|&t| t == now_ms) {
                v.remove(pos);
            }
        }
    }
}

impl Default for UploadRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

use crate::auth::session::AuthUser;
use crate::data::asset::process::audio::{self, AudioContainers};
use crate::data::asset::process::{derivative_path, sibling_paths, write_derivatives, Variant};
use crate::data::asset::tags::{derive, DeriveInput};
use crate::data::asset::{
    commit_staged_asset, move_asset_files, process_staged_blocking, remove_asset_files, Asset,
    Provenance,
};
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};
use crate::ws::protocol::{AssetOp, ServerMsg};
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::io::AsyncWriteExt;

/// Stream a multipart "file" field to `dest`, enforcing `max_bytes` as bytes
/// arrive (never buffering the whole body). Returns
/// `(content_type, byte_size, original_name, containers)`, where `content_type` is the
/// type SNIFFED from the leading bytes when they are a supported image; when
/// they are not, the client's declared type is used as a plain label —
/// unless it CLAIMS `image/*`, which the bytes just disproved, in which case
/// the label is `application/octet-stream`. The bytes are the validation
/// boundary; a client's image claim is never trusted. `containers` is the
/// optional trailing text field selecting the audio derivative container(s).
/// On any failure the partial file is removed.
async fn store_streamed(
    mut multipart: Multipart,
    dest: &std::path::Path,
    max_bytes: u64,
) -> Result<(String, i64, String, Option<String>), AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| AppError::BadRequest("missing file field".into()))?;
    let original_name = field.file_name().unwrap_or("upload").to_string();
    let declared = field.content_type().map(str::to_string);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|_| AppError::Internal)?;
    // Leading bytes for the sniff; `detect_image_type` needs at most 12.
    let mut head: Vec<u8> = Vec::with_capacity(16);
    let mut total: u64 = 0;

    let mut field = field;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(AppError::BadRequest(format!("multipart error: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > max_bytes {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::PayloadTooLarge(format!(
                "file exceeds {max_bytes} bytes"
            )));
        }
        if head.len() < 12 {
            let want = 12 - head.len();
            head.extend_from_slice(&chunk[..chunk.len().min(want)]);
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|_| AppError::Internal)?;

    // An optional trailing `containers` text field selects the audio derivative container(s)
    // (`data::asset::process::audio::AudioContainers`'s snake_case names); the file field is
    // always first, so anything after it that is not this field is ignored.
    let mut containers: Option<String> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("containers") {
            containers = field.text().await.ok().map(|s| s.trim().to_string());
        }
    }

    let content_type = label_content_type_with_audio(
        detect_image_type(&head),
        detect_audio_type(&head),
        declared.as_deref(),
    );
    Ok((content_type, total as i64, original_name, containers))
}

/// The content type recorded for an upload: the sniffed image type when the
/// bytes are a supported image, else the sniffed audio type when the bytes are
/// a recognized audio container; otherwise the declared type as a label,
/// except that a declared `image/*` the bytes disproved becomes
/// `application/octet-stream`.
pub(super) fn label_content_type(sniffed: Option<&'static str>, declared: Option<&str>) -> String {
    label_content_type_with_audio(sniffed, None, declared)
}

/// `label_content_type` with the audio sniff supplied (kept separate so the image-only
/// callers' signature is untouched). The BYTES win over the declared label for both media
/// families: a mislabeled audio upload (`application/octet-stream` on a real WAV) is
/// classified audio and reaches the transcode arm, and a declared `image/*` the bytes
/// disproved stays `application/octet-stream` (a browser must never be told a non-image is
/// an image).
pub(super) fn label_content_type_with_audio(
    sniffed_image: Option<&'static str>,
    sniffed_audio: Option<&'static str>,
    declared: Option<&str>,
) -> String {
    if let Some(ct) = sniffed_image {
        return ct.to_string();
    }
    if let Some(ct) = sniffed_audio {
        return ct.to_string();
    }
    match declared {
        Some(d) if !d.starts_with("image/") && !d.is_empty() => d.to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

/// Parse the optional `containers` multipart field into an audio derivative selection
/// (absent/empty ⇒ the default). The snake_case spellings are
/// `AudioContainers`'s own serde names — one statement of the accepted set, beside the one
/// multipart reader that produces the raw string.
fn parse_containers_field(value: Option<&str>) -> Result<AudioContainers, AppError> {
    match value {
        None | Some("") => Ok(AudioContainers::default()),
        Some("ogg") => Ok(AudioContainers::Ogg),
        Some("webm") => Ok(AudioContainers::WebM),
        Some("both") => Ok(AudioContainers::Both),
        Some(other) => Err(AppError::BadRequest(format!(
            "unknown containers '{other}'"
        ))),
    }
}

/// `POST /api/worlds/{world}/assets` — GM-gated single-shot multipart upload
/// (`require_gm`; server admins resolve to GM). There is no owner exception.
/// Images are converted through `data::asset::process`; anything else is
/// stored pass-through under its declared type. Lands in the world root.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::password::hash_password;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::config::Config;
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::{self, AppState};
/// use std::sync::{atomic::AtomicBool, Arc};
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let hash = hash_password("pw").unwrap();
/// let gm = repo.create_user("gm", Some(&hash), ServerRole::User, 0).await.unwrap();
/// let world = repo.create_world_owned("example", gm, 0).await.unwrap();
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
/// };
/// let server = axum_test::TestServer::builder()
///     .save_cookies()
///     .build(http::router(state).await)
///     .unwrap();
/// server
///     .post("/api/login")
///     .json(&serde_json::json!({ "username": "gm", "password": "pw" }))
///     .await;
/// let response = server
///     .post(&format!("/api/worlds/{}/assets", world.id))
///     .multipart(
///         axum_test::multipart::MultipartForm::new().add_part(
///             "file",
///             axum_test::multipart::Part::bytes(b"\x89PNG\r\n\x1a\n".to_vec())
///                 .file_name("dot.png")
///                 .mime_type("image/png"),
///         ),
///     )
///     .await;
/// response.assert_status_ok();
/// # }
/// ```
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let ctx = require_gm(&state, &user, world).await?;
    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(
        user.id,
        now,
        state.config.effective_rate_per_min(ctx.world_role),
    ) {
        return Err(AppError::TooManyRequests(
            "upload rate limit exceeded".into(),
        ));
    }
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world}/{id}");
    let final_path = state
        .config
        .assets_path()
        .join(world.to_string())
        .join(id.to_string());
    // Unique temp sibling in the same directory: stream network bytes to disk
    // BEFORE acquiring the backup quiesce barrier below. These routes disable
    // `DefaultBodyLimit`, so a slow multipart upload has no timeout — holding
    // a write-preferring `tokio::sync::RwLock`'s read side across that wait
    // would queue an admin `write()` (i.e. `/api/admin/backup`) behind it.
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    let max = state.config.effective_max_bytes(ctx.world_role);

    // Do the fallible work in one block so a failure at any step refunds the
    // rate-limit hit `check` recorded — a rejected upload must not burn quota.
    let retain = state.config.retain_originals;
    let outcome: Result<Asset, AppError> = async {
        let (arrived_type, arrived_size, original_name, containers_field) =
            store_streamed(multipart, &tmp_path, max).await?;
        let containers = parse_containers_field(containers_field.as_deref())?;
        // CPU-bound conversion, off the async runtime and BEFORE the barrier.
        let processed = process_staged_blocking(
            tmp_path.clone(),
            arrived_type,
            arrived_size,
            retain,
            containers,
        )
        .await
        .map_err(|e| {
            tracing::error!(?e, %id, "asset processing failed");
            AppError::Internal
        })?;
        // Single-shot uploads land in the world root: no folder segments.
        let derived = derive(DeriveInput {
            content_type: &processed.content_type,
            meta: &processed.meta,
            folder_names: &[],
            provenance: Provenance::Uploaded,
        });
        let asset = Asset {
            id,
            world_id: world,
            storage_key,
            original_name,
            content_type: processed.content_type,
            byte_size: processed.byte_size,
            created_by: Some(user.id),
            created_at: now,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: processed.meta,
        };
        // Read-side of the backup quiesce barrier, acquired only around the
        // rename+DB-commit pair below — the one critical section the quiesce
        // exists to keep non-interleaving with an in-server backup's VACUUM +
        // assets copy. Concurrent asset writes share the read side freely;
        // this serializes nothing between uploads.
        let _read_permit = state.write_barrier.read().await;
        commit_staged_asset(&state.repo, &tmp_path, &final_path, asset, &derived)
            .await
            .map_err(AppError::from)
    }
    .await;

    match outcome {
        Ok(asset) => {
            if let Some(room) = state.ws.rooms.get(world) {
                room.broadcast_aux(ServerMsg::AssetChanged {
                    uuid: asset.id,
                    op: AssetOp::Created,
                    version: asset.version,
                });
            }
            Ok(Json(asset))
        }
        Err(e) => {
            state.upload_rate.refund(user.id, now);
            Err(e)
        }
    }
}

/// `?variant=` on `GET /api/assets/{uuid}`.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::ServeQuery;
///
/// let q: ServeQuery = serde_json::from_str(r#"{"variant":"thumb"}"#).unwrap();
/// assert_eq!(q.variant.as_deref(), Some("thumb"));
/// let canonical: ServeQuery = serde_json::from_str("{}").unwrap();
/// assert!(canonical.variant.is_none()); // absent = the canonical file
/// ```
#[derive(Debug, serde::Deserialize)]
pub struct ServeQuery {
    /// `thumb` | `preview` | `opus` | `opus-webm`; absent = the canonical file.
    pub variant: Option<String>,
}

/// `GET /api/assets/{uuid}[?variant=thumb|preview]` — read-gated by world
/// membership; ETag-revalidated. A derivative shares the canonical's ETag
/// (`"{id}-{version}"`): it is regenerated whenever the canonical's version
/// changes, so the version keys it. A missing derivative is regenerated on
/// demand; if the canonical does not decode, the canonical itself is served
/// in its place rather than a 404.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "member-example".into(), role: ServerRole::User };
/// let _ = shadowcat::http::assets::serve(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
///     axum::extract::Query(shadowcat::http::assets::ServeQuery { variant: None }),
///     axum::http::HeaderMap::new(),
/// )
/// .await;
/// # }
/// ```
pub async fn serve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ServeQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // The Opus derivatives are NOT `Variant`s and bypass `ensure_derivative` entirely: a
    // minutes-long transcode is never regenerated on serve, so a missing sibling is a plain
    // 404 — derivatives are produced at commit time or not at all.
    if matches!(q.variant.as_deref(), Some("opus") | Some("opus-webm")) {
        let (suffix, content_type, etag_suffix) = if q.variant.as_deref() == Some("opus") {
            (audio::OPUS_SUFFIX, audio::OPUS_CONTENT_TYPE, "opus")
        } else {
            (audio::WEBM_SUFFIX, audio::WEBM_CONTENT_TYPE, "opus-webm")
        };
        let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
        // Same read-gate as the canonical path: any member of the asset's world may read.
        state
            .repo
            .permission_context(asset.world_id, user.id, user.role)
            .await?;
        let canonical = state.config.assets_path().join(&asset.storage_key);
        let sibling = crate::data::asset::process::with_suffix(&canonical, suffix);
        let bytes = tokio::fs::read(&sibling)
            .await
            .map_err(|_| AppError::NotFound)?;
        let etag = format!("\"{id}-{}-{etag_suffix}\"", asset.version);
        let if_none_match = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if if_none_match.split(',').any(|t| t.trim() == etag) {
            return Ok((StatusCode::NOT_MODIFIED).into_response());
        }
        // `inline` is safe here regardless of `INLINE_CONTENT_TYPES`'s raster-only scope: an
        // audio derivative embeds via `<audio src>`, never `<img>` or a navigation.
        return Ok((
            [
                (header::CONTENT_TYPE, content_type.to_string()),
                (header::CONTENT_DISPOSITION, "inline".to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::ETAG, etag),
            ],
            Body::from(bytes),
        )
            .into_response());
    }
    let variant = match q.variant.as_deref() {
        None => None,
        Some("thumb") => Some(Variant::Thumb),
        Some("preview") => Some(Variant::Preview),
        Some(other) => {
            return Err(AppError::BadRequest(format!("unknown variant '{other}'")));
        }
    };
    let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    // Read-gate: any member of the asset's world may read. permission_context
    // returns Forbidden for non-members.
    state
        .repo
        .permission_context(asset.world_id, user.id, user.role)
        .await?;

    let etag = format!("\"{}-{}\"", id, asset.version);
    // `If-None-Match` is an RFC 7232 comma-separated list (browsers may send
    // several cached ETags); 304 if ours appears anywhere in it.
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if if_none_match.split(',').any(|t| t.trim() == etag) {
        return Ok((StatusCode::NOT_MODIFIED).into_response());
    }

    let canonical = state.config.assets_path().join(&asset.storage_key);
    let (path, content_type) = match variant {
        None => (canonical, asset.content_type),
        Some(v) => match ensure_derivative(&canonical, v).await {
            Ok(p) => (
                p,
                crate::data::asset::process::WEBP_CONTENT_TYPE.to_string(),
            ),
            Err(e) => {
                // Not decodable (pass-through non-image, corrupt file): the
                // canonical stands in for its own preview.
                tracing::debug!(?e, %id, "derivative unavailable; serving canonical");
                (canonical, asset.content_type)
            }
        },
    };
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        tracing::error!(?e, %id, "asset file missing for existing record");
        AppError::Internal
    })?;
    // Pass-through stores a GM-declared type verbatim, and `serve` is
    // membership-gated, so the response never lets a browser render an
    // asset as a document: `nosniff` always, and `inline` only for the raster
    // types a browser cannot execute — everything else (SVG included, which
    // can carry script when navigated to directly) downloads. `<img>`
    // embedding is unaffected by the disposition.
    let disposition = if INLINE_CONTENT_TYPES.contains(&content_type.as_str()) {
        "inline"
    } else {
        "attachment"
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::ETAG, etag),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// Content types `serve` presents `inline`: raster images a browser can only
/// paint. Every other stored type is served as an attachment.
const INLINE_CONTENT_TYPES: [&str; 6] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
    "image/tiff",
];

/// Path of the `variant` derivative of `canonical`, regenerating both
/// derivatives (blocking pool) when it is missing.
async fn ensure_derivative(
    canonical: &std::path::Path,
    variant: Variant,
) -> std::io::Result<std::path::PathBuf> {
    let path = derivative_path(canonical, variant);
    if tokio::fs::try_exists(&path).await? {
        return Ok(path);
    }
    let src = canonical.to_path_buf();
    tokio::task::spawn_blocking(move || write_derivatives(&src))
        .await
        .map_err(std::io::Error::other)??;
    Ok(path)
}

/// `POST /api/assets/{uuid}/replace` — GM-gated byte-swap behind a stable id
/// (`require_gm`; no owner exception). Undo-exempt: no world seq, no
/// event-log entry.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::password::hash_password;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::config::Config;
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::{self, AppState};
/// use std::sync::{atomic::AtomicBool, Arc};
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let hash = hash_password("pw").unwrap();
/// let gm = repo.create_user("gm", Some(&hash), ServerRole::User, 0).await.unwrap();
/// let world = repo.create_world_owned("example", gm, 0).await.unwrap();
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
/// };
/// let server = axum_test::TestServer::builder()
///     .save_cookies()
///     .build(http::router(state).await)
///     .unwrap();
/// server
///     .post("/api/login")
///     .json(&serde_json::json!({ "username": "gm", "password": "pw" }))
///     .await;
/// let created = server
///     .post(&format!("/api/worlds/{}/assets", world.id))
///     .multipart(
///         axum_test::multipart::MultipartForm::new().add_part(
///             "file",
///             axum_test::multipart::Part::bytes(b"\x89PNG\r\n\x1a\n".to_vec())
///                 .file_name("dot.png")
///                 .mime_type("image/png"),
///         ),
///     )
///     .await
///     .json::<shadowcat::data::asset::Asset>();
/// let response = server
///     .post(&format!("/api/assets/{}/replace", created.id))
///     .multipart(
///         axum_test::multipart::MultipartForm::new().add_part(
///             "file",
///             axum_test::multipart::Part::bytes(b"\x89PNG\r\n\x1a\n".to_vec())
///                 .file_name("dot2.png")
///                 .mime_type("image/png"),
///         ),
///     )
///     .await;
/// response.assert_status_ok();
/// # }
/// ```
pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    let ctx = require_gm(&state, &user, existing.world_id).await?;
    // Replace streams a full file like upload, so it shares the per-user tiered
    // rate limit — the cap is on total write volume, not per-endpoint.
    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(
        user.id,
        now,
        state.config.effective_rate_per_min(ctx.world_role),
    ) {
        return Err(AppError::TooManyRequests(
            "replace rate limit exceeded".into(),
        ));
    }

    // Stream the new bytes to a per-request temp file. A unique name (not a
    // fixed `<uuid>.tmp`) keeps two concurrent replaces of the same asset from
    // clobbering each other's partial writes.
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    let max = state.config.effective_max_bytes(ctx.world_role);

    // Fallible work in one block so any failure refunds the rate-limit hit `check`
    // recorded — a rejected replace must not burn quota.
    let retain = state.config.retain_originals;
    let outcome: Result<Asset, AppError> = async {
        let (arrived_type, arrived_size, _name, containers_field) =
            store_streamed(multipart, &tmp_path, max).await?;
        // No explicit selection on a replace re-emits the derivative set the asset already
        // has (`audio::effective_reencode_selection`), never silently widening or narrowing it.
        let containers = match containers_field.as_deref() {
            Some(_) => parse_containers_field(containers_field.as_deref())?,
            None => audio::effective_reencode_selection(
                audio::has_sibling(&final_path, audio::OPUS_SUFFIX),
                audio::has_sibling(&final_path, audio::WEBM_SUFFIX),
            ),
        };
        let processed = process_staged_blocking(
            tmp_path.clone(),
            arrived_type,
            arrived_size,
            retain,
            containers,
        )
        .await
        .map_err(|e| {
            tracing::error!(?e, %id, "asset processing failed");
            AppError::Internal
        })?;
        commit_replacement(&state, &existing, &tmp_path, &final_path, processed).await
    }
    .await;

    match outcome {
        Ok(asset) => Ok(Json(asset)),
        Err(e) => {
            state.upload_rate.refund(user.id, now);
            Err(e)
        }
    }
}

/// The shared tail of every byte-swap behind a stable id (`replace`,
/// `mutate::reconvert`): row-first commit, then canonical + sibling swap,
/// then derived-tag refresh and the `Replaced` broadcast. `processed` is the
/// pipeline's verdict on the bytes staged at `tmp_path`.
///
/// Read-side of the backup quiesce barrier is acquired only around the
/// DB-commit + rename pair — the one critical section the quiesce exists to
/// keep non-interleaving with an in-server backup's VACUUM + assets copy —
/// never across the caller's network-bound stream or CPU-bound conversion:
/// a slow uploader holding a write-preferring `tokio::sync::RwLock`'s read
/// side open would queue an admin `write()` behind it indefinitely.
///
/// Commits to the DB BEFORE swapping the live file. If the DB write fails the
/// live bytes are untouched and the record stays consistent (tmp is removed).
/// If the rename later fails, the DB is one version ahead of unchanged bytes
/// — clients re-fetch (ETag changed) and the next replace lands correctly;
/// the inverse order would strand new bytes under a stale ETag (broken 304)
/// [[commit-db-row-before-swapping-file]].
pub(super) async fn commit_replacement(
    state: &AppState,
    existing: &Asset,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    processed: crate::data::asset::process::Processed,
) -> Result<Asset, AppError> {
    let id = existing.id;
    let _read_permit = state.write_barrier.read().await;
    let version = match state
        .repo
        .replace_asset_bytes(
            id,
            &existing.storage_key,
            &processed.content_type,
            processed.byte_size,
            &processed.meta,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            remove_asset_files(tmp_path).await;
            return Err(e.into());
        }
    };
    // Canonical + every sibling (a stale `.orig`/derivative of the old
    // bytes is removed when the new upload has none).
    if let Err(e) = move_asset_files(tmp_path, final_path).await {
        remove_asset_files(tmp_path).await;
        tracing::error!(?e, %id, "asset replace rename failed after DB commit");
        return Err(AppError::Internal);
    }
    // Kind/dimension/alpha tags follow the new bytes.
    state.repo.refresh_derived_tags(id).await?;

    if let Some(room) = state.ws.rooms.get(existing.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Replaced,
            version,
        });
    }

    state
        .repo
        .get_asset(id)
        .await?
        .ok_or(AppError::NotFound)
        .map(|a| Asset { version, ..a })
}

/// Remove asset `id`'s row, its canonical + sibling files, and broadcast
/// `Deleted` — the single delete tail shared by the `DELETE /api/assets/{uuid}`
/// route and the folder purge. Holds the read side of the backup quiesce
/// barrier across the row-removal + unlink pair: without it a backup could
/// capture the row gone but the file still present (or vice versa), and in
/// the row-gone/file-still-present ordering the backup's manifest would
/// reference a file the DB no longer knows about. Returns `false` when a
/// racing delete already removed the row (nothing left to unlink or
/// broadcast — that delete already did both). Authorization is the caller's.
pub(super) async fn delete_asset_files_and_row(
    state: &AppState,
    id: uuid::Uuid,
) -> Result<bool, AppError> {
    let _read_permit = state.write_barrier.read().await;
    let Some(deleted) = state.repo.delete_asset(id).await? else {
        return Ok(false);
    };
    let path = state.config.assets_path().join(&deleted.storage_key);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        // Record is gone; a missing file is not fatal (it becomes a no-op).
        tracing::warn!(?e, %id, "asset file remove failed after record delete");
    }
    // Siblings (`.orig`, derivatives) exist only for some assets; absence is
    // the ordinary case and not worth a warning.
    for sibling in sibling_paths(&path) {
        match tokio::fs::remove_file(&sibling).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(?e, %id, "asset sibling remove failed after record delete"),
        }
    }
    if let Some(room) = state.ws.rooms.get(deleted.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Deleted,
            version: deleted.version,
        });
    }
    Ok(true)
}

/// `DELETE /api/assets/{uuid}` — GM-gated (`require_gm`; no owner exception).
/// Undo-exempt.
///
/// `existing` (the pre-delete read) backs only `require_gm`'s authorization and the initial
/// `NotFound` check: `write_barrier`'s read side excludes a backup's write side, not a racing
/// `replace` on the same id, so `existing.version` can be stale by the time the row is actually
/// removed — `delete_asset_files_and_row` reads the row `DELETE ... RETURNING *` actually removed
/// for every post-delete use, so the broadcast always carries the truly deleted version.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let _ = shadowcat::http::assets::delete(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
/// )
/// .await;
/// # }
/// ```
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    require_gm(&state, &user, existing.world_id).await?;
    delete_asset_files_and_row(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
