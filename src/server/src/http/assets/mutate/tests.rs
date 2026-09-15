use super::*;

#[test]
fn attachment_disposition_strips_header_breaking_characters() {
    assert_eq!(
        attachment_disposition("ma\"p\\.png\r\nX-Injected: 1"),
        "attachment; filename=\"map.pngX-Injected: 1\""
    );
    assert_eq!(
        attachment_disposition("plain.png"),
        "attachment; filename=\"plain.png\""
    );
}

/// A live `AppState` (in-memory repo, temp assets dir) with a GM-seated world,
/// for exercising `patch`'s sheet-pairing validation directly.
struct PairingFixture {
    /// Keeps the temp assets dir alive for the fixture's lifetime.
    _dir: tempfile::TempDir,
    /// The constructed app state.
    state: AppState,
    /// The GM user's id (also the world's creator).
    gm: Uuid,
    /// The world every fixture asset belongs to.
    world: Uuid,
}

impl PairingFixture {
    /// Builds the state, seats a GM, and creates the world.
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
        let state = AppState {
            repo,
            config: std::sync::Arc::new(config),
            setup_token: None,
            initialized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ws: crate::ws::WsState::new(),
            upload_rate: std::sync::Arc::new(crate::http::assets::UploadRateLimiter::new()),
            uploads: std::sync::Arc::new(crate::http::assets::uploads::UploadSessions::new()),
            auth_throttle: std::sync::Arc::new(crate::http::throttle::AuthThrottle::new()),
            write_barrier: std::sync::Arc::new(tokio::sync::RwLock::new(())),
            preview_fetch_locks: std::sync::Arc::new(dashmap::DashMap::new()),
        };
        PairingFixture {
            _dir: dir,
            state,
            gm,
            world: world.id,
        }
    }

    /// Inserts an asset row (metadata only; no bytes on disk) and returns its id.
    async fn insert_asset(&self, original_name: &str, content_type: &str) -> Uuid {
        let id = Uuid::new_v4();
        let asset = crate::data::asset::Asset {
            id,
            world_id: self.world,
            storage_key: format!("{}/{id}", self.world),
            original_name: original_name.into(),
            content_type: content_type.into(),
            byte_size: 10,
            created_by: Some(self.gm),
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed(content_type, 10),
        };
        self.state.repo.insert_asset(&asset).await.unwrap();
        id
    }

    /// Writes `bytes` as the canonical file of the asset `id` on disk.
    fn write_bytes(&self, id: Uuid, bytes: &[u8]) {
        let path = self.state.config.assets_path().join(self.world.to_string());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(id.to_string()), bytes).unwrap();
    }

    /// Runs `patch` with a replacement tag set as the GM.
    async fn patch_tags(&self, id: Uuid, tags: Vec<String>) -> Result<(), AppError> {
        let user = crate::auth::session::AuthUser {
            id: self.gm,
            username: "gm".into(),
            role: crate::auth::role::ServerRole::User,
        };
        let body = Json(PatchAssetRequest {
            name: None,
            folder_id: None,
            tags: Some(tags),
        });
        patch(State(self.state.clone()), user, Path(id), body)
            .await
            .map(|_| ())
    }
}

#[tokio::test]
async fn patch_pairs_a_valid_spritesheet_sidecar() {
    let f = PairingFixture::new().await;
    let image = f.insert_asset("map.png", "image/webp").await;
    let json = f.insert_asset("map.json", "application/json").await;
    f.write_bytes(json, br#"{"meta":{"image":"map.png"}}"#);
    f.patch_tags(image, vec![format!("vfx:sheet={json}")])
        .await
        .unwrap();
    let updated = f.state.repo.get_asset(image).await.unwrap().unwrap();
    assert_eq!(updated.tags, vec![format!("vfx:sheet={json}")]);
}

#[tokio::test]
async fn patch_refuses_a_sidecar_whose_meta_image_names_another_file() {
    let f = PairingFixture::new().await;
    let image = f.insert_asset("other.png", "image/webp").await;
    let json = f.insert_asset("map.json", "application/json").await;
    f.write_bytes(json, br#"{"meta":{"image":"map.png"}}"#);
    let res = f.patch_tags(image, vec![format!("vfx:sheet={json}")]).await;
    assert!(matches!(res, Err(AppError::Unprocessable(_))), "{res:?}");
}

#[tokio::test]
async fn patch_refuses_a_nonexistent_sidecar_id_and_a_second_sheet_tag() {
    let f = PairingFixture::new().await;
    let image = f.insert_asset("map.png", "image/webp").await;
    let res = f
        .patch_tags(image, vec![format!("vfx:sheet={}", Uuid::new_v4())])
        .await;
    assert!(matches!(res, Err(AppError::Unprocessable(_))), "{res:?}");
    let json = f.insert_asset("map.json", "application/json").await;
    f.write_bytes(json, br#"{"meta":{"image":"map.png"}}"#);
    // Two DISTINCT `vfx:sheet=` tags — an identical pair is one tag after
    // `normalize_tags`' dedupe, and one pairing is exactly what the rule allows.
    let res = f
        .patch_tags(
            image,
            vec![
                format!("vfx:sheet={json}"),
                format!("vfx:sheet={}", Uuid::new_v4()),
            ],
        )
        .await;
    assert!(matches!(res, Err(AppError::Unprocessable(_))), "{res:?}");
}

#[tokio::test]
async fn patch_refuses_a_non_json_sidecar_and_an_unreadable_sidecar_file() {
    let f = PairingFixture::new().await;
    let image = f.insert_asset("map.png", "image/webp").await;
    // Right id, wrong content type.
    let not_json = f.insert_asset("map.png", "image/webp").await;
    let res = f
        .patch_tags(image, vec![format!("vfx:sheet={not_json}")])
        .await;
    assert!(matches!(res, Err(AppError::Unprocessable(_))), "{res:?}");
    // Right type, but the canonical file was never written.
    let json = f.insert_asset("map.json", "application/json").await;
    let res = f.patch_tags(image, vec![format!("vfx:sheet={json}")]).await;
    assert!(matches!(res, Err(AppError::Unprocessable(_))), "{res:?}");
}

#[tokio::test]
async fn patch_without_a_sheet_tag_is_unaffected_by_the_new_validation() {
    let f = PairingFixture::new().await;
    let image = f.insert_asset("map.png", "image/webp").await;
    f.patch_tags(image, vec!["vfx".into()]).await.unwrap();
    let updated = f.state.repo.get_asset(image).await.unwrap().unwrap();
    assert_eq!(updated.tags, vec!["vfx".to_string()]);
}
