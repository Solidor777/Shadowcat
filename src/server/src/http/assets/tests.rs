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

/// A live `AppState` (in-memory repo, temp assets dir) with a GM-seated world and one asset
/// whose canonical and grid-sheet sibling are on disk, for exercising `serve`'s
/// `?variant=sheet` arm directly.
struct SheetFixture {
    /// Keeps the temp assets dir alive for the fixture's lifetime.
    _dir: tempfile::TempDir,
    /// The constructed app state.
    state: AppState,
    /// The seeded asset's id.
    asset: Uuid,
    /// The seeded world's id.
    world: Uuid,
    /// The GM user's id (also the world's creator).
    gm: Uuid,
}

impl SheetFixture {
    /// Builds the state, seats a GM, creates the world, and seeds one asset row plus its
    /// canonical and `.sheet.webp` sibling bytes.
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            assets_dir: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        let repo = std::sync::Arc::new(
            crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
                .await
                .unwrap(),
        );
        let gm = repo
            .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("w", gm, 0).await.unwrap();
        let asset = Uuid::new_v4();
        repo.insert_asset(&crate::data::asset::Asset {
            id: asset,
            world_id: world.id,
            storage_key: format!("{}/{asset}", world.id),
            original_name: "fx.webp".into(),
            content_type: "image/webp".into(),
            byte_size: 10,
            created_by: Some(gm),
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/webp", 10),
        })
        .await
        .unwrap();
        let dir_path = dir.path().join(world.id.to_string());
        std::fs::create_dir_all(&dir_path).unwrap();
        std::fs::write(dir_path.join(asset.to_string()), b"canonical").unwrap();
        std::fs::write(
            crate::data::asset::process::sheet_path(&dir_path.join(asset.to_string())),
            b"sheet-bytes",
        )
        .unwrap();
        let state = AppState {
            repo,
            config: std::sync::Arc::new(config),
            setup_token: None,
            initialized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ws: crate::ws::WsState::new(),
            upload_rate: std::sync::Arc::new(UploadRateLimiter::new()),
            uploads: std::sync::Arc::new(crate::http::assets::uploads::UploadSessions::new()),
            auth_throttle: std::sync::Arc::new(crate::http::throttle::AuthThrottle::new()),
            write_barrier: std::sync::Arc::new(tokio::sync::RwLock::new(())),
            preview_fetch_locks: std::sync::Arc::new(dashmap::DashMap::new()),
        };
        SheetFixture {
            _dir: dir,
            state,
            asset,
            world: world.id,
            gm,
        }
    }

    /// Calls `serve` for asset `id` with `?variant=sheet`, as the fixture GM.
    async fn serve_sheet(&self, id: Uuid) -> Result<Response, AppError> {
        let user = crate::auth::session::AuthUser {
            id: self.gm,
            username: "gm".into(),
            role: crate::auth::role::ServerRole::User,
        };
        serve(
            State(self.state.clone()),
            user,
            Path(id),
            Query(ServeQuery {
                variant: Some("sheet".into()),
            }),
            HeaderMap::new(),
        )
        .await
    }
}

#[tokio::test]
async fn serve_variant_sheet_serves_the_grid_sheet_sibling() {
    let f = SheetFixture::new().await;
    let res = f.serve_sheet(f.asset).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE).unwrap(),
        crate::data::asset::process::WEBP_CONTENT_TYPE
    );
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert_eq!(&body[..], b"sheet-bytes");
}

#[tokio::test]
async fn serve_variant_sheet_is_not_found_without_the_sibling() {
    let f = SheetFixture::new().await;
    // A second asset whose row exists but whose sheet sibling was never written: 404,
    // never the canonical standing in (the canonical is the animated source, not a sheet).
    let other = Uuid::new_v4();
    let world = f.world;
    f.state
        .repo
        .insert_asset(&crate::data::asset::Asset {
            id: other,
            world_id: world,
            storage_key: format!("{world}/{other}"),
            original_name: "fx2.webp".into(),
            content_type: "image/webp".into(),
            byte_size: 10,
            created_by: None,
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/webp", 10),
        })
        .await
        .unwrap();
    let res = f.serve_sheet(other).await;
    assert!(matches!(res, Err(AppError::NotFound)), "{res:?}");
    // And an unknown variant name is still a bad request, not a sheet lookup.
    let user = crate::auth::session::AuthUser {
        id: Uuid::new_v4(),
        username: "gm".into(),
        role: crate::auth::role::ServerRole::User,
    };
    let bad = serve(
        State(f.state.clone()),
        user,
        Path(f.asset),
        Query(ServeQuery {
            variant: Some("bogus".into()),
        }),
        HeaderMap::new(),
    )
    .await;
    assert!(matches!(bad, Err(AppError::BadRequest(_))), "{bad:?}");
}
