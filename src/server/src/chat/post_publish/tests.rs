use super::*;
use crate::auth::role::ServerRole;
use crate::chat::{
    build_link_preview_client, handle_delete_message, handle_send_message, Audience,
    LinkPreviewCache, LinkPreviewDeps, MessageRequestCtx, PreviewRateLimiter,
};
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;
use crate::ws::PingRateLimiter;
use axum::routing::get;
use axum::Router;

async fn spawn_stub(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (addr.to_string(), handle)
}

/// A fresh, uncontended write-barrier for tests exercising the asset
/// commit path — no test in this module holds the write side, so the
/// read permit `resolve_preview_image` acquires never blocks.
fn test_write_barrier() -> Arc<tokio::sync::RwLock<()>> {
    Arc::new(tokio::sync::RwLock::new(()))
}

/// The lock key of `url` in the world every lock-registry test below shares.
fn lock_key(url: &str) -> (Uuid, String) {
    (crate::test_ids::test_uuid(1), url.to_string())
}

/// A fresh, empty per-URL fetch-lock registry for tests exercising
/// `resolve_preview_image`/`resolve_thumbnail_asset` directly.
fn test_preview_fetch_locks() -> PreviewFetchLocks {
    Arc::new(DashMap::new())
}

/// Builds `FetchDeps` for a direct resolver call -- a fresh, uncontended
/// write-barrier plus the caller-supplied lock registry (so a test can
/// share one registry across several calls to observe cleanup/racing
/// behavior, or pass a fresh `test_preview_fetch_locks()` for isolation).
fn test_fetch_deps(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    preview_fetch_locks: PreviewFetchLocks,
) -> FetchDeps {
    FetchDeps {
        repo,
        client,
        assets_root,
        retain_originals: true,
        write_barrier: test_write_barrier(),
        preview_fetch_locks,
    }
}

/// A publish helper: sends a plain message and returns its message id --
/// no `og:image` involved (these tests exercise `run_pending_enrichments`
/// directly, not the synchronous scrape). Seeds the channel-registry
/// singleton first when the world lacks one: the send path refuses an
/// unregistered channel, and these directly-built worlds never pass through
/// create/join seeding.
async fn seed_message(room: &Room, repo: &SqliteRepository, ctx: &PermissionContext) -> Uuid {
    if repo
        .query_documents(
            room.world_id,
            crate::data::engine::CHANNEL_REGISTRY_DOC_TYPE,
        )
        .await
        .unwrap()
        .is_empty()
    {
        crate::data::world_seed::seed_test_channel_registry(repo, room.world_id, &[]).await;
    }
    let rate = PingRateLimiter::new();
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room,
            repo,
            ctx,
            rate: &rate,
            vfx_rate: &rate,

            preview: LinkPreviewDeps {
                client: &build_link_preview_client(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "general".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap()
    .unwrap();
    match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    }
}

#[tokio::test]
async fn resolve_preview_image_cache_hit_skips_network_fetch() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let asset = crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: world.id,
        storage_key: "x".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 3,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    let preview_url = "https://cached.example/";
    repo.upsert_link_preview_cache(world.id, preview_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(world.id, preview_url, asset.id)
        .await
        .unwrap();

    // A client pointed at nothing reachable: if the cache hit is skipped
    // and a real fetch is attempted, this would hang/fail rather than
    // resolve immediately.
    let unreachable_client = Arc::new(build_link_preview_client());
    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo,
            unreachable_client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![preview_url.to_string()],
        preview_url.to_string(),
        String::new(),
        String::new(),
        "https://cached.example/og.png".to_string(),
    )
    .await
    .expect("cache hit must resolve without a network fetch");
    match resolved {
        ResolvedEnrichment::ImageForPreview { asset_id, .. } => {
            assert_eq!(asset_id, asset.id);
        }
        other => panic!("expected ImageForPreview, got {other:?}"),
    }
}

#[tokio::test]
async fn resolve_preview_image_cache_miss_fetches_and_creates_asset() {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![9u8, 9, 9],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();

    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let preview_url = "https://miss.example/";
    let image_url = format!("http://stub.test:{port}/img.png");
    // The row must already exist: `enrich`'s synchronous title/description
    // scrape always upserts a row for the previewed URL BEFORE this
    // background job ever runs (production invariant this test mirrors) --
    // `set_link_preview_cache_image` alone is a no-op on an absent row.
    repo.upsert_link_preview_cache(world.id, preview_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();

    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![preview_url.to_string()],
        preview_url.to_string(),
        String::new(),
        String::new(),
        image_url,
    )
    .await
    .expect("cache miss must fetch and create an asset");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    let asset = repo.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset.created_by, None);
    assert_eq!(asset.byte_size, 3);
    let row = repo
        .get_link_preview_cache(world.id, preview_url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(asset_id));
}

#[tokio::test]
async fn resolve_preview_image_after_a_redirect_persists_and_dedups_on_the_cache_url() {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![7u8, 7, 7],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let cache_url = "https://posted.example/short";
    let preview_url = "https://landed.example/final";
    repo.upsert_link_preview_cache(world.id, cache_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();

    let first = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![cache_url.to_string()],
        preview_url.to_string(),
        String::new(),
        String::new(),
        format!("http://stub.test:{port}/img.png"),
    )
    .await
    .expect("a miss must fetch and create an asset");
    let ResolvedEnrichment::ImageForPreview {
        preview_url: patched,
        asset_id,
    } = first
    else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(
        patched, preview_url,
        "the segment patch targets the post-redirect url"
    );
    let row = repo
        .get_link_preview_cache(world.id, cache_url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(asset_id));
    let landed = repo
        .get_link_preview_cache(world.id, preview_url)
        .await
        .unwrap()
        .expect("the landed address row is created");
    assert_eq!(landed.image_asset_id, Some(asset_id));

    // A second message reaching the same candidate hits the persisted row and never fetches:
    // the client below cannot reach any image host.
    let second = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            Arc::new(build_link_preview_client()),
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![cache_url.to_string()],
        preview_url.to_string(),
        String::new(),
        String::new(),
        "https://unreachable.example/img.png".to_string(),
    )
    .await
    .expect("a persisted hit resolves without a fetch");
    let ResolvedEnrichment::ImageForPreview {
        asset_id: reused, ..
    } = second
    else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(reused, asset_id);
}

#[tokio::test]
async fn resolve_inline_image_cache_miss_fetches_creates_a_chat_image_asset() {
    let router = Router::new().route(
        "/pic.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![4u8, 4, 4],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();

    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let image_url = format!("http://stub.test:{port}/pic.png");

    let resolved = resolve_inline_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        image_url.clone(),
        "a map".to_string(),
    )
    .await
    .expect("cache miss must fetch and create a chat-image asset");
    let ResolvedEnrichment::NewImageSegment(Segment::Image { asset_id, alt }) = resolved else {
        panic!("expected a new Image segment");
    };
    assert_eq!(alt, "a map");
    let asset = repo.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset.created_by, None);
    assert_eq!(asset.byte_size, 3);
    assert_eq!(asset.original_name, "pic.png");
    assert_eq!(
        crate::data::asset::tags::provenance_of(&asset.derived_tags),
        crate::data::asset::Provenance::ChatImage
    );
    let row = repo
        .get_link_preview_cache(world.id, &image_url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(asset_id));
}

#[tokio::test]
async fn resolve_inline_image_cache_hit_skips_network_fetch() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let asset = crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: world.id,
        storage_key: "x".into(),
        original_name: "pic.png".into(),
        content_type: "image/png".into(),
        byte_size: 3,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    let image_url = "https://cached.example/pic.png";
    repo.upsert_link_preview_cache(world.id, image_url, None, None, 0)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(world.id, image_url, asset.id)
        .await
        .unwrap();

    // A client pointed at nothing reachable: a cache-hit skip is proven by
    // resolving without hanging/erroring on a real fetch attempt.
    let unreachable_client = Arc::new(build_link_preview_client());
    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_inline_image(
        test_fetch_deps(
            repo,
            unreachable_client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        image_url.to_string(),
        "cached alt".to_string(),
    )
    .await
    .expect("cache hit must resolve without a network fetch");
    let ResolvedEnrichment::NewImageSegment(Segment::Image { asset_id, alt }) = resolved else {
        panic!("expected a new Image segment");
    };
    assert_eq!(asset_id, asset.id);
    assert_eq!(alt, "cached alt");
}

#[tokio::test]
async fn resolve_inline_image_refuses_a_body_over_max_inline_image_bytes() {
    let router = Router::new().route(
        "/big.png",
        get(|| async {
            let body = vec![0u8; MAX_INLINE_IMAGE_BYTES + 1024];
            ([(axum::http::header::CONTENT_TYPE, "image/png")], body)
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let image_url = format!("http://stub.test:{port}/big.png");

    let resolved = resolve_inline_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        image_url,
        "too big".to_string(),
    )
    .await;
    assert!(
        resolved.is_none(),
        "a body over MAX_INLINE_IMAGE_BYTES must be refused, not asset-ified"
    );
    assert!(repo
        .list_assets_by_world(world.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn run_pending_enrichments_appends_new_image_segment_for_an_inline_image_job() {
    let router = Router::new().route(
        "/pic.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![2u8, 2, 2],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;

    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let jobs = vec![PendingEnrichment::InlineImage {
        image_url: format!("http://stub.test:{port}/pic.png"),
        alt: "a map".to_string(),
    }];
    let root = tempfile::tempdir().unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client,
            assets_root: root.path().to_path_buf(),
            retain_originals: true,
            write_barrier: test_write_barrier(),
            preview_fetch_locks: test_preview_fetch_locks(),
        },
        message_id,
        world.id,
        jobs,
    )
    .await;

    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    let found = sys
        .content
        .iter()
        .find_map(|s| match s {
            Segment::Image { asset_id, alt } => Some((*asset_id, alt.clone())),
            _ => None,
        })
        .expect("expected an appended Image segment");
    assert_eq!(found.1, "a map");
    let asset = repo.get_asset(found.0).await.unwrap().unwrap();
    assert_eq!(
        crate::data::asset::tags::provenance_of(&asset.derived_tags),
        crate::data::asset::Provenance::ChatImage
    );
}

#[tokio::test]
async fn run_pending_enrichments_patches_matching_preview_by_url() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;

    // Manually inject two LinkPreview segments (bypassing the
    // synchronous scrape, which this test does not exercise).
    let cur = repo.get_document(message_id).await.unwrap().unwrap();
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap()).unwrap();
    sys.content.push(Segment::LinkPreview {
        url: "https://match.example/".into(),
        title: "M".into(),
        description: "".into(),
        image_asset_id: None,
    });
    sys.content.push(Segment::LinkPreview {
        url: "https://other.example/".into(),
        title: "O".into(),
        description: "".into(),
        image_asset_id: None,
    });
    let new_engine = serde_json::to_value(&sys).unwrap();
    room.publish(
        repo.as_ref(),
        &ctx,
        vec![Operation::Update {
            doc_id: message_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur.engine.unwrap(),
                new: new_engine,
            }],
        }],
        101,
        WriteOrigin::ServerMessageRevision,
    )
    .await
    .unwrap();

    let asset_id = Uuid::new_v4();
    let jobs = vec![PendingEnrichment::PreviewImage {
        cache_urls: vec!["https://match.example/".into()],
        preview_url: "https://match.example/".into(),
        title: String::new(),
        description: String::new(),
        image_url: "https://match.example/og.png".into(),
    }];
    // Pre-seed the persisted cache row so resolution never touches the
    // network -- this test targets the patch-by-url logic, not the fetch.
    repo.upsert_link_preview_cache(world.id, "https://match.example/", None, None, 0)
        .await
        .unwrap();
    let asset = crate::data::asset::Asset {
        id: asset_id,
        world_id: world.id,
        storage_key: "y".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image(world.id, "https://match.example/", asset_id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
            retain_originals: true,
            write_barrier: test_write_barrier(),
            preview_fetch_locks: test_preview_fetch_locks(),
        },
        message_id,
        world.id,
        jobs,
    )
    .await;

    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    let mut found_match = None;
    let mut found_other = None;
    for seg in &sys.content {
        if let Segment::LinkPreview {
            url,
            image_asset_id,
            ..
        } = seg
        {
            if url == "https://match.example/" {
                found_match = Some(*image_asset_id);
            } else if url == "https://other.example/" {
                found_other = Some(*image_asset_id);
            }
        }
    }
    assert_eq!(found_match, Some(Some(asset_id)));
    assert_eq!(found_other, Some(None));
}

#[tokio::test]
async fn run_pending_enrichments_is_a_noop_on_tombstoned_message() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;
    let rate = PingRateLimiter::new();
    handle_delete_message(&room, repo.as_ref(), &ctx, &rate, message_id, 101, 30)
        .await
        .unwrap();

    let seq_before = room.subscribe().1;
    let jobs = vec![PendingEnrichment::PreviewImage {
        cache_urls: vec!["https://match.example/".into()],
        preview_url: "https://match.example/".into(),
        title: String::new(),
        description: String::new(),
        image_url: "https://match.example/og.png".into(),
    }];
    repo.upsert_link_preview_cache(world.id, "https://match.example/", None, None, 0)
        .await
        .unwrap();
    let asset = crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: world.id,
        storage_key: "z".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image(world.id, "https://match.example/", asset.id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
            retain_originals: true,
            write_barrier: test_write_barrier(),
            preview_fetch_locks: test_preview_fetch_locks(),
        },
        message_id,
        world.id,
        jobs,
    )
    .await;

    assert_eq!(
        room.subscribe().1,
        seq_before,
        "a tombstoned message must not be re-published"
    );
}

#[tokio::test]
async fn run_pending_enrichments_reads_fresh_content_for_occ() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;

    // Inject the target LinkPreview segment.
    let cur = repo.get_document(message_id).await.unwrap().unwrap();
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap()).unwrap();
    sys.content.push(Segment::LinkPreview {
        url: "https://match.example/".into(),
        title: "M".into(),
        description: "".into(),
        image_asset_id: None,
    });
    let new_engine = serde_json::to_value(&sys).unwrap();
    room.publish(
        repo.as_ref(),
        &ctx,
        vec![Operation::Update {
            doc_id: message_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur.engine.unwrap(),
                new: new_engine,
            }],
        }],
        101,
        WriteOrigin::ServerMessageRevision,
    )
    .await
    .unwrap();

    // A concurrent write appends OTHER content between the job's
    // resolution and `run_pending_enrichments`' own re-read -- a raw
    // `Operation::Update` (not `handle_edit_message`, which fully
    // replaces `content` from a re-parsed body and would wipe the
    // injected LinkPreview segment above, defeating this test's setup).
    let cur2 = repo.get_document(message_id).await.unwrap().unwrap();
    let mut sys2: MessageEngine = serde_json::from_value(cur2.engine.clone().unwrap()).unwrap();
    sys2.content.push(Segment::Text {
        text: "concurrent addition".into(),
    });
    let new_engine2 = serde_json::to_value(&sys2).unwrap();
    room.publish(
        repo.as_ref(),
        &ctx,
        vec![Operation::Update {
            doc_id: message_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur2.engine.unwrap(),
                new: new_engine2,
            }],
        }],
        102,
        WriteOrigin::ServerMessageRevision,
    )
    .await
    .unwrap();

    let asset_id = Uuid::new_v4();
    let jobs = vec![PendingEnrichment::PreviewImage {
        cache_urls: vec!["https://match.example/".into()],
        preview_url: "https://match.example/".into(),
        title: String::new(),
        description: String::new(),
        image_url: "https://match.example/og.png".into(),
    }];
    repo.upsert_link_preview_cache(world.id, "https://match.example/", None, None, 0)
        .await
        .unwrap();
    let asset = crate::data::asset::Asset {
        id: asset_id,
        world_id: world.id,
        storage_key: "w".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image(world.id, "https://match.example/", asset_id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
            retain_originals: true,
            write_barrier: test_write_barrier(),
            preview_fetch_locks: test_preview_fetch_locks(),
        },
        message_id,
        world.id,
        jobs,
    )
    .await;

    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    // The concurrent addition survives (the OCC pre-image read was
    // fresh, not stale from before it).
    assert!(sys
        .content
        .iter()
        .any(|s| matches!(s, Segment::Text { text } if text == "concurrent addition")));
    // AND the image patch landed alongside it.
    let patched = sys.content.iter().find_map(|s| match s {
        Segment::LinkPreview {
            url,
            image_asset_id,
            ..
        } if url == "https://match.example/" => Some(*image_asset_id),
        _ => None,
    });
    assert_eq!(patched, Some(Some(asset_id)));
}

#[tokio::test]
async fn run_pending_enrichments_appends_new_oembed_segment() {
    // `OEmbedProvider::endpoint` is fixed to a real provider host
    // (https://www.youtube.com/oembed | https://vimeo.com/api/oembed.json)
    // and is not stubbable directly. This test instead builds the exact
    // `Segment::OEmbed` `resolve_oembed` would, via its constituent
    // pieces (`fetch_json_bytes`/`resolve_thumbnail_asset`) run against a
    // stub JSON+image server addressed through the SAME
    // `build_client_with_resolve_fn` seam `link_preview.rs`'s own tests
    // use (a fake hostname resolved to the stub's real loopback
    // address), then feeds the resulting `ResolvedEnrichment` through
    // `publish_resolved` -- `run_pending_enrichments`'s own
    // OCC-read/patch/publish tail, factored out precisely so this test
    // can reach it without a live fetch against the fixed provider host.
    let img_router = Router::new().route(
        "/thumb.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![7u8, 7, 7],
            )
        }),
    );
    let (img_addr, _img_handle) = spawn_stub(img_router).await;
    let img_port: u16 = img_addr.rsplit(':').next().unwrap().parse().unwrap();
    let thumbnail_url = format!("http://stub.test:{img_port}/thumb.png");

    let json_body = format!(
        r#"{{"title":"A Video","author_name":"Someone","thumbnail_url":"{thumbnail_url}","html":"<script>alert(1)</script>"}}"#
    );
    let json_router = Router::new().route(
        "/oembed.json",
        get(move || {
            let body = json_body.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
            }
        }),
    );
    let (json_addr, _json_handle) = spawn_stub(json_router).await;
    let json_port: u16 = json_addr.rsplit(':').next().unwrap().parse().unwrap();

    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;
    let root = tempfile::tempdir().unwrap();

    let body = fetch_json_bytes(
        &client,
        &format!("http://stub.test:{json_port}/oembed.json"),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    let parsed: OEmbedResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("A Video"));

    let thumbnail_asset_id = resolve_thumbnail_asset(
        test_fetch_deps(
            repo.clone(),
            client.clone(),
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        parsed.thumbnail_url.clone().unwrap(),
    )
    .await;
    assert!(
        thumbnail_asset_id.is_some(),
        "the stub thumbnail must resolve to a created asset"
    );

    let segment = Segment::OEmbed(OEmbedSegment {
        url: "https://www.youtube.com/watch?v=abc".into(),
        provider_name: OEmbedProvider::YouTube.name().to_string(),
        title: parsed.title,
        author_name: parsed.author_name,
        thumbnail_asset_id,
    });
    // Assembled downstream of the provider's raw `html` field, which
    // never crossed the `OEmbedResponse` boundary into `OEmbedSegment`.
    let serialized = serde_json::to_string(&segment).unwrap();
    assert!(!serialized.contains("<script"));

    publish_resolved(
        room.clone(),
        repo.clone(),
        message_id,
        world.id,
        vec![ResolvedEnrichment::NewOEmbedSegment(segment)],
    )
    .await;

    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    let found = sys.content.iter().find_map(|s| match s {
        Segment::OEmbed(oe) => Some(oe.clone()),
        _ => None,
    });
    let oe = found.expect("expected an appended OEmbed segment");
    assert_eq!(oe.provider_name, "YouTube");
    assert_eq!(oe.title.as_deref(), Some("A Video"));
    assert!(oe.thumbnail_asset_id.is_some());
}

/// Confirms `preview_fetch_locks` serializes two simultaneous
/// `resolve_preview_image` calls for the IDENTICAL `preview_url`, racing
/// against a real cache-miss fetch, into exactly one `Asset` row, with
/// both calls resolving to the same `asset_id`. Without that
/// serialization, `resolve_preview_image`'s cache check and its eventual
/// `set_link_preview_cache_image` write are not mutually exclusive
/// against a concurrent caller's own check, so nothing stops two
/// independent `create_asset_from_bytes` calls for the one URL. The stub
/// image handler sleeps before responding specifically so both spawned
/// tasks are genuinely in flight at once -- a lock that merely happened
/// to run the two calls one after another with no real overlap would not
/// exercise this.
#[tokio::test]
async fn resolve_preview_image_concurrent_requests_for_same_url_create_one_asset() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fetch_count = Arc::new(AtomicUsize::new(0));
    let fetch_count_for_route = fetch_count.clone();
    let router = Router::new().route(
        "/img.png",
        get(move || {
            let fetch_count = fetch_count_for_route.clone();
            async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                // Held open long enough that an unserialized second
                // caller would reach this handler too, before the first
                // caller's asset commit + cache write land.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                (
                    [(axum::http::header::CONTENT_TYPE, "image/png")],
                    vec![5u8, 5, 5],
                )
            }
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();

    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let preview_url = "https://race.example/".to_string();
    let image_url = format!("http://stub.test:{port}/img.png");
    // Mirrors the production invariant: `enrich`'s synchronous scrape
    // always upserts the row before a background job runs.
    repo.upsert_link_preview_cache(world.id, &preview_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();

    let locks = test_preview_fetch_locks();

    let h1 = tokio::spawn({
        let repo = repo.clone();
        let client = client.clone();
        let root = root.path().to_path_buf();
        let locks = locks.clone();
        let world_id = world.id;
        let preview_url = preview_url.clone();
        let image_url = image_url.clone();
        async move {
            resolve_preview_image(
                test_fetch_deps(repo, client, root, locks),
                world_id,
                vec![preview_url.clone()],
                preview_url,
                String::new(),
                String::new(),
                image_url,
            )
            .await
        }
    });
    let h2 = tokio::spawn({
        let repo = repo.clone();
        let client = client.clone();
        let root = root.path().to_path_buf();
        let locks = locks.clone();
        let world_id = world.id;
        let preview_url = preview_url.clone();
        let image_url = image_url.clone();
        async move {
            resolve_preview_image(
                test_fetch_deps(repo, client, root, locks),
                world_id,
                vec![preview_url.clone()],
                preview_url,
                String::new(),
                String::new(),
                image_url,
            )
            .await
        }
    });

    let (r1, r2) = tokio::join!(h1, h2);
    let r1 = r1.unwrap().expect("first concurrent resolve must succeed");
    let r2 = r2.unwrap().expect("second concurrent resolve must succeed");
    let ResolvedEnrichment::ImageForPreview { asset_id: id1, .. } = r1 else {
        panic!("expected ImageForPreview");
    };
    let ResolvedEnrichment::ImageForPreview { asset_id: id2, .. } = r2 else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(
        id1, id2,
        "both concurrent resolves for the identical URL must land on the same asset_id"
    );
    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        1,
        "the per-URL lock must serialize the two concurrent resolves into exactly one network fetch"
    );

    let assets = repo.list_assets_by_world(world.id).await.unwrap();
    assert_eq!(
        assets.len(),
        1,
        "exactly one Asset row must exist for the raced URL, not one per racing caller"
    );

    // The registry itself must not retain the URL's lock once both
    // resolvers have completed -- see `with_preview_url_locks`' cleanup
    // reasoning.
    assert!(
        locks.is_empty(),
        "the per-URL lock entry must be reclaimed once no resolver still needs it"
    );
}

/// The lock-registry map must not grow unboundedly across many distinct
/// URLs -- each entry is reclaimed once its resolver completes, not left
/// to accumulate for the life of the process.
#[tokio::test]
async fn preview_fetch_locks_does_not_retain_entries_after_resolvers_complete() {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![1u8, 2, 3],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let locks = test_preview_fetch_locks();

    for i in 0..25 {
        let preview_url = format!("https://distinct-{i}.example/");
        repo.upsert_link_preview_cache(world.id, &preview_url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
        let image_url = format!("http://stub.test:{port}/img.png");
        resolve_preview_image(
            test_fetch_deps(
                repo.clone(),
                client.clone(),
                root.path().to_path_buf(),
                locks.clone(),
            ),
            world.id,
            vec![preview_url.clone()],
            preview_url,
            String::new(),
            String::new(),
            image_url,
        )
        .await
        .expect("each distinct URL must resolve");
    }

    assert!(
        locks.is_empty(),
        "the lock registry must not retain an entry per distinct URL once each resolver \
         has completed -- it would otherwise grow unboundedly over a long-running server's \
         lifetime"
    );
}

/// Appends `segments` to the stored message `message_id` through the same
/// `WriteOrigin::ServerMessageRevision` republish the pipeline itself uses.
async fn append_segments(
    room: &Room,
    repo: &SqliteRepository,
    ctx: &PermissionContext,
    message_id: Uuid,
    segments: Vec<Segment>,
) {
    let cur = repo.get_document(message_id).await.unwrap().unwrap();
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap()).unwrap();
    sys.content.extend(segments);
    room.publish(
        repo,
        ctx,
        vec![Operation::Update {
            doc_id: message_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur.engine.unwrap(),
                new: serde_json::to_value(&sys).unwrap(),
            }],
        }],
        101,
        WriteOrigin::ServerMessageRevision,
    )
    .await
    .unwrap();
}

/// Two candidate URLs whose previews ended on one address ride a single job: the run fetches the
/// image once, creates one asset, patches the segments that carry the shared `preview_url`, and
/// records the asset id on the `link_preview_cache` row of every candidate.
#[tokio::test]
async fn run_pending_enrichments_serves_candidates_sharing_a_preview_url_with_one_asset() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fetch_count = Arc::new(AtomicUsize::new(0));
    let fetch_count_for_route = fetch_count.clone();
    let router = Router::new().route(
        "/img.png",
        get(move || {
            let fetch_count = fetch_count_for_route.clone();
            async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                (
                    [(axum::http::header::CONTENT_TYPE, "image/png")],
                    vec![4u8, 4, 4],
                )
            }
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Gm,
    };
    let message_id = seed_message(&room, &repo, &ctx).await;

    let cache_urls = vec![
        "https://first.example/short".to_string(),
        "https://second.example/other".to_string(),
    ];
    let preview_url = "https://landed.example/final/";
    let preview_segment = |title: &str| Segment::LinkPreview {
        url: preview_url.into(),
        title: title.into(),
        description: String::new(),
        image_asset_id: None,
    };
    append_segments(
        &room,
        &repo,
        &ctx,
        message_id,
        vec![preview_segment("A"), preview_segment("B")],
    )
    .await;
    for cache_url in &cache_urls {
        repo.upsert_link_preview_cache(world.id, cache_url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }

    let root = tempfile::tempdir().unwrap();
    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
                |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
            )),
            assets_root: root.path().to_path_buf(),
            retain_originals: true,
            write_barrier: test_write_barrier(),
            preview_fetch_locks: test_preview_fetch_locks(),
        },
        message_id,
        world.id,
        vec![PendingEnrichment::PreviewImage {
            cache_urls: cache_urls.clone(),
            preview_url: preview_url.into(),
            title: String::new(),
            description: String::new(),
            image_url: format!("http://stub.test:{port}/img.png"),
        }],
    )
    .await;

    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        1,
        "one job fetches the image once"
    );
    let assets = repo.list_assets_by_world(world.id).await.unwrap();
    assert_eq!(assets.len(), 1, "one job creates one asset");
    for cache_url in &cache_urls {
        let row = repo
            .get_link_preview_cache(world.id, cache_url)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.image_asset_id,
            Some(assets[0].id),
            "the row of {cache_url} receives the asset id"
        );
    }
    let landed = repo
        .get_link_preview_cache(world.id, preview_url)
        .await
        .unwrap()
        .expect("the preview url row is created");
    assert_eq!(
        landed.image_asset_id,
        Some(assets[0].id),
        "the preview url row receives the asset id"
    );
    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    let patched: Vec<Option<Uuid>> = sys
        .content
        .iter()
        .filter_map(|seg| match seg {
            Segment::LinkPreview { image_asset_id, .. } => Some(*image_asset_id),
            _ => None,
        })
        .collect();
    assert_eq!(patched, vec![Some(assets[0].id), Some(assets[0].id)]);
}

/// A job whose rows partly carry an asset id reuses it without fetching and writes it onto the
/// rows that lack one.
#[tokio::test]
async fn resolve_preview_image_propagates_a_known_asset_id_to_the_candidates_lacking_one() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let asset = crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: world.id,
        storage_key: "k".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();
    let known = "https://known.example/a";
    let lacking = "https://lacking.example/b";
    for url in [known, lacking] {
        repo.upsert_link_preview_cache(world.id, url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }
    repo.set_link_preview_cache_image(world.id, known, asset.id)
        .await
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            Arc::new(build_link_preview_client()),
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![lacking.to_string(), known.to_string()],
        "https://landed.example/".to_string(),
        String::new(),
        String::new(),
        "https://unreachable.example/img.png".to_string(),
    )
    .await
    .expect("a known asset id resolves without a fetch");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(asset_id, asset.id);
    let row = repo
        .get_link_preview_cache(world.id, lacking)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(asset.id));
}

/// Inserts an asset row in `world_id` and returns its id.
async fn insert_test_asset(repo: &SqliteRepository, world_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    repo.insert_asset(&crate::data::asset::Asset {
        id,
        world_id,
        storage_key: format!("{world_id}/{id}"),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    })
    .await
    .unwrap();
    id
}

/// An asset id another writer records on the job's row while the job fetches stays: the job's
/// own asset does not replace it.
#[tokio::test]
async fn resolve_preview_image_does_not_overwrite_an_id_recorded_while_it_fetched() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let rival = insert_test_asset(&repo, world.id).await;
    let cache_url = "https://posted.example/page";
    repo.upsert_link_preview_cache(world.id, cache_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();

    // The image response records `rival` on the row before answering, the way a concurrent
    // writer that does not hold the job's lock would.
    let rival_repo = repo.clone();
    let router = Router::new().route(
        "/img.png",
        get(move || {
            let repo = rival_repo.clone();
            async move {
                repo.set_link_preview_cache_image(world.id, cache_url, rival)
                    .await
                    .unwrap();
                (
                    [(axum::http::header::CONTENT_TYPE, "image/png")],
                    vec![7u8, 7, 7],
                )
            }
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![cache_url.to_string()],
        cache_url.to_string(),
        String::new(),
        String::new(),
        format!("http://stub.test:{port}/img.png"),
    )
    .await
    .expect("a miss fetches and creates an asset");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    assert_ne!(asset_id, rival, "the job created its own asset");
    let row = repo
        .get_link_preview_cache(world.id, cache_url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(rival));
}

/// A job whose candidate redirected to an address that has its own row carrying an asset id
/// reuses that asset without fetching and records it on the candidate's row.
#[tokio::test]
async fn resolve_preview_image_reuses_an_asset_recorded_on_the_landed_address_row() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let known = insert_test_asset(&repo, world.id).await;
    let candidate = "https://posted.example/short";
    let landed = "https://landed.example/final";
    for url in [candidate, landed] {
        repo.upsert_link_preview_cache(world.id, url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }
    repo.set_link_preview_cache_image(world.id, landed, known)
        .await
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            Arc::new(build_link_preview_client()),
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![candidate.to_string()],
        landed.to_string(),
        String::new(),
        String::new(),
        "https://unreachable.example/img.png".to_string(),
    )
    .await
    .expect("an asset on the landed address row resolves without a fetch");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(asset_id, known);
    let row = repo
        .get_link_preview_cache(world.id, candidate)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(known));
}

/// A job that creates an asset records it on the landed address row too, when that row exists
/// without an asset id.
#[tokio::test]
async fn resolve_preview_image_records_a_new_asset_on_a_landed_address_row_without_one() {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![7u8, 7, 7],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let candidate = "https://posted.example/short";
    let landed = "https://landed.example/final";
    for url in [candidate, landed] {
        repo.upsert_link_preview_cache(world.id, url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let root = tempfile::tempdir().unwrap();
    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world.id,
        vec![candidate.to_string()],
        landed.to_string(),
        String::new(),
        String::new(),
        format!("http://stub.test:{port}/img.png"),
    )
    .await
    .expect("a miss fetches and creates an asset");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    for url in [candidate, landed] {
        let row = repo
            .get_link_preview_cache(world.id, url)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.image_asset_id, Some(asset_id), "{url}");
    }
}

/// A job whose landed address has no row creates it from the preview the job carries and records
/// the new asset on it, so a later post of the landed address reuses that asset without a fetch.
#[tokio::test]
async fn resolve_preview_image_creates_the_landed_address_row_that_a_later_post_reuses() {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![7u8, 7, 7],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let candidate = "https://posted.example/short";
    let landed = "https://landed.example/final";
    repo.upsert_link_preview_cache(world.id, candidate, Some("T"), Some("D"), 0)
        .await
        .unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    let root = tempfile::tempdir().unwrap();
    let locks = test_preview_fetch_locks();
    let first = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            locks.clone(),
        ),
        world.id,
        vec![candidate.to_string()],
        landed.to_string(),
        "MOCK_TITLE".to_string(),
        "MOCK_DESCRIPTION".to_string(),
        format!("http://stub.test:{port}/img.png"),
    )
    .await
    .expect("a miss fetches and creates an asset");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = first else {
        panic!("expected ImageForPreview");
    };
    let row = repo
        .get_link_preview_cache(world.id, landed)
        .await
        .unwrap()
        .expect("the landed address row exists");
    assert_eq!(row.title.as_deref(), Some("MOCK_TITLE"));
    assert_eq!(row.description.as_deref(), Some("MOCK_DESCRIPTION"));
    assert_eq!(row.image_asset_id, Some(asset_id));

    let later = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            Arc::new(build_link_preview_client()),
            root.path().to_path_buf(),
            locks,
        ),
        world.id,
        vec![landed.to_string()],
        landed.to_string(),
        String::new(),
        String::new(),
        "https://unreachable.example/img.png".to_string(),
    )
    .await
    .expect("the landed row's asset resolves without a fetch");
    let ResolvedEnrichment::ImageForPreview {
        asset_id: reused, ..
    } = later
    else {
        panic!("expected ImageForPreview");
    };
    assert_eq!(reused, asset_id);
    assert_eq!(repo.list_assets_by_world(world.id).await.unwrap().len(), 1);
}

/// Callers whose key sets overlap and list the shared keys in opposite orders all complete, and
/// the registry is empty afterwards.
#[tokio::test]
async fn with_preview_url_locks_overlapping_key_sets_in_opposite_orders_do_not_deadlock() {
    let locks = test_preview_fetch_locks();
    let mut tasks = Vec::new();
    for i in 0..16 {
        let locks = locks.clone();
        tasks.push(tokio::spawn(async move {
            let keys = if i % 2 == 0 {
                vec![lock_key("a"), lock_key("b"), lock_key("c")]
            } else {
                vec![lock_key("c"), lock_key("b"), lock_key("a")]
            };
            with_preview_url_locks(&locks, &keys, || async {
                tokio::task::yield_now().await;
            })
            .await;
        }));
    }
    let all = async {
        for task in tasks {
            task.await.unwrap();
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), all)
        .await
        .expect("overlapping key sets must not deadlock");
    assert!(
        locks.is_empty(),
        "every entry is reclaimed once all callers finish"
    );
}

/// A repeated key in one caller's key list is locked once, so the caller does not deadlock on
/// itself, and the registry is empty afterwards.
#[tokio::test]
async fn with_preview_url_locks_duplicate_keys_lock_once_and_reclaim_the_entry() {
    let locks = test_preview_fetch_locks();
    let keys = vec![lock_key("a"), lock_key("a")];
    let run = with_preview_url_locks(&locks, &keys, || async { 7 });
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), run)
        .await
        .expect("a duplicated key must not deadlock the caller on itself");
    assert_eq!(out, 7);
    assert!(locks.is_empty(), "the single entry is reclaimed");
}

/// Releasing a key removes its entry exactly when the releaser is the last holder: a releaser
/// that still sees another holder leaves the entry, and the holder that releases afterwards
/// removes it.
#[tokio::test]
async fn release_preview_lock_removes_the_entry_only_for_the_last_holder() {
    let locks = test_preview_fetch_locks();
    let caller = locks
        .entry(lock_key("a"))
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let other = locks.get(&lock_key("a")).unwrap().clone();
    release_preview_lock(&locks, lock_key("a"), caller);
    assert_eq!(locks.len(), 1, "another holder remains, so the entry stays");
    release_preview_lock(&locks, lock_key("a"), other);
    assert!(
        locks.is_empty(),
        "the last holder to release removes the entry"
    );
}

/// The same URL in two worlds is two keys: one caller's hold does not block the other's.
#[tokio::test]
async fn with_preview_url_locks_same_url_in_different_worlds_does_not_contend() {
    let locks = test_preview_fetch_locks();
    let url = "https://shared.example/".to_string();
    let first = (crate::test_ids::test_uuid(1), url.clone());
    let second = (crate::test_ids::test_uuid(2), url);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let holder = {
        let locks = locks.clone();
        let first = first.clone();
        tokio::spawn(async move {
            with_preview_url_locks(&locks, &[first], || async {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
            })
            .await;
        })
    };
    entered_rx.await.unwrap();
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        with_preview_url_locks(&locks, &[second], || async { 7 }),
    )
    .await
    .expect("the other world's key is not held");
    assert_eq!(out, 7);
    release_tx.send(()).unwrap();
    holder.await.unwrap();
    assert!(locks.is_empty());
}

/// A caller whose future is dropped inside `f` releases its entry.
#[tokio::test]
async fn with_preview_url_locks_dropped_inside_f_reclaims_the_entry() {
    let locks = test_preview_fetch_locks();
    let keys = vec![lock_key("a")];
    let run = with_preview_url_locks(&locks, &keys, std::future::pending::<()>);
    tokio::time::timeout(std::time::Duration::from_millis(50), run)
        .await
        .expect_err("f never completes, so the timeout drops the future");
    assert!(locks.is_empty(), "the dropped caller's entry is reclaimed");
}

/// A caller whose future is dropped while it awaits a later key releases the keys it already
/// holds, and the entry another caller still holds stays until that caller releases it.
#[tokio::test]
async fn with_preview_url_locks_dropped_while_awaiting_a_key_reclaims_the_held_keys() {
    let locks = test_preview_fetch_locks();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let holder = {
        let locks = locks.clone();
        tokio::spawn(async move {
            with_preview_url_locks(&locks, &[lock_key("b")], || async {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
            })
            .await;
        })
    };
    entered_rx.await.unwrap();
    let keys = vec![lock_key("a"), lock_key("b")];
    let waiter = with_preview_url_locks(&locks, &keys, || async {});
    tokio::time::timeout(std::time::Duration::from_millis(50), waiter)
        .await
        .expect_err("key b is held, so the timeout drops the waiting future");
    assert!(
        !locks.contains_key(&lock_key("a")),
        "the key the dropped caller held is reclaimed"
    );
    assert!(
        locks.contains_key(&lock_key("b")),
        "the other caller still holds b"
    );
    release_tx.send(()).unwrap();
    holder.await.unwrap();
    assert!(locks.is_empty(), "the remaining holder removes its entry");
}

/// Seeds two worlds on one repository and serves a PNG from a stub host; returns the repository,
/// the two worlds, a client that resolves the stub host, and the stub's port and task handle.
async fn two_worlds_and_a_png_stub() -> (
    Arc<SqliteRepository>,
    crate::data::document::World,
    crate::data::document::World,
    Arc<reqwest::Client>,
    u16,
    tokio::task::JoinHandle<()>,
) {
    let router = Router::new().route(
        "/img.png",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![5u8, 5, 5],
            )
        }),
    );
    let (addr, handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world_a = repo.create_world_owned("a", owner, 0).await.unwrap();
    let world_b = repo.create_world_owned("b", owner, 0).await.unwrap();
    let client = Arc::new(crate::chat::link_preview::build_client_with_resolve_fn(
        |_host| Ok(vec!["127.0.0.1".parse().unwrap()]),
    ));
    (repo, world_a, world_b, client, port, handle)
}

/// A preview image another world already recorded for the address is not reused: the job creates
/// its own asset in its own world.
#[tokio::test]
async fn resolve_preview_image_does_not_reuse_another_worlds_asset() {
    let (repo, world_a, world_b, client, port, _stub) = two_worlds_and_a_png_stub().await;
    let root = tempfile::tempdir().unwrap();
    let url = "https://shared.example/page";
    let foreign = insert_test_asset(&repo, world_a.id).await;
    for world in [&world_a, &world_b] {
        repo.upsert_link_preview_cache(world.id, url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }
    repo.set_link_preview_cache_image(world_a.id, url, foreign)
        .await
        .unwrap();

    let resolved = resolve_preview_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world_b.id,
        vec![url.to_string()],
        url.to_string(),
        "T".to_string(),
        "D".to_string(),
        format!("http://stub.test:{port}/img.png"),
    )
    .await
    .expect("the job resolves with an asset of its own");
    let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved else {
        panic!("expected ImageForPreview");
    };
    assert_ne!(asset_id, foreign);
    let asset = repo.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset.world_id, world_b.id);
    let row_a = repo.get_link_preview_cache(world_a.id, url).await.unwrap();
    assert_eq!(row_a.unwrap().image_asset_id, Some(foreign));
    let row_b = repo.get_link_preview_cache(world_b.id, url).await.unwrap();
    assert_eq!(row_b.unwrap().image_asset_id, Some(asset_id));
}

/// An inline image another world already recorded for the address is not reused.
#[tokio::test]
async fn resolve_inline_image_does_not_reuse_another_worlds_asset() {
    let (repo, world_a, world_b, client, port, _stub) = two_worlds_and_a_png_stub().await;
    let root = tempfile::tempdir().unwrap();
    let image_url = format!("http://stub.test:{port}/img.png");
    let foreign = insert_test_asset(&repo, world_a.id).await;
    repo.upsert_link_preview_cache(world_a.id, &image_url, None, None, 0)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(world_a.id, &image_url, foreign)
        .await
        .unwrap();

    let resolved = resolve_inline_image(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world_b.id,
        image_url,
        "alt".to_string(),
    )
    .await
    .expect("the job resolves with an asset of its own");
    let ResolvedEnrichment::NewImageSegment(Segment::Image { asset_id, .. }) = resolved else {
        panic!("expected a new Image segment");
    };
    assert_ne!(asset_id, foreign);
    let asset = repo.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset.world_id, world_b.id);
}

/// An oEmbed thumbnail another world already recorded for the address is not reused.
#[tokio::test]
async fn resolve_thumbnail_asset_does_not_reuse_another_worlds_asset() {
    let (repo, world_a, world_b, client, port, _stub) = two_worlds_and_a_png_stub().await;
    let root = tempfile::tempdir().unwrap();
    let thumbnail_url = format!("http://stub.test:{port}/img.png");
    let foreign = insert_test_asset(&repo, world_a.id).await;
    repo.upsert_link_preview_cache(world_a.id, &thumbnail_url, None, None, 0)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(world_a.id, &thumbnail_url, foreign)
        .await
        .unwrap();

    let asset_id = resolve_thumbnail_asset(
        test_fetch_deps(
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_preview_fetch_locks(),
        ),
        world_b.id,
        thumbnail_url,
    )
    .await
    .expect("the job resolves with an asset of its own");
    assert_ne!(asset_id, foreign);
    let asset = repo.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset.world_id, world_b.id);
}

/// Deleting a world removes its cache rows and leaves another world's rows, asset ids included.
#[tokio::test]
async fn deleting_a_world_leaves_another_worlds_cache_rows() {
    let (repo, world_a, world_b, _client, _port, _stub) = two_worlds_and_a_png_stub().await;
    let url = "https://shared.example/page";
    let kept = insert_test_asset(&repo, world_b.id).await;
    for world in [&world_a, &world_b] {
        repo.upsert_link_preview_cache(world.id, url, Some("T"), Some("D"), 0)
            .await
            .unwrap();
    }
    repo.set_link_preview_cache_image(world_b.id, url, kept)
        .await
        .unwrap();

    repo.delete_world(world_a.id).await.unwrap();

    assert_eq!(
        repo.get_link_preview_cache(world_a.id, url).await.unwrap(),
        None
    );
    let row = repo
        .get_link_preview_cache(world_b.id, url)
        .await
        .unwrap()
        .expect("the other world's row stays");
    assert_eq!(row.image_asset_id, Some(kept));
}

/// Holds the fetch lock of `key` until the returned sender fires or drops.
async fn hold_fetch_lock(
    locks: &PreviewFetchLocks,
    key: PreviewLockKey,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let locks = locks.clone();
    let handle = tokio::spawn(async move {
        with_preview_url_locks(&locks, &[key], || async {
            entered_tx.send(()).unwrap();
            let _ = release_rx.await;
        })
        .await;
    });
    entered_rx.await.unwrap();
    (release_tx, handle)
}

/// Each resolver takes the lock of its URL in its own world: the lock held for the same URL in the
/// job's world blocks the job, and the lock held for that URL in another world does not.
#[tokio::test]
async fn resolvers_lock_their_url_in_their_own_world() {
    let (repo, world_a, world_b, client, port, _stub) = two_worlds_and_a_png_stub().await;
    let root = tempfile::tempdir().unwrap();
    let locks = test_preview_fetch_locks();
    let deps = || {
        test_fetch_deps(
            repo.clone(),
            client.clone(),
            root.path().to_path_buf(),
            locks.clone(),
        )
    };
    let image_url = format!("http://stub.test:{port}/img.png");
    let blocked = std::time::Duration::from_millis(200);
    let open = std::time::Duration::from_secs(10);

    for (holder_world, window, completes) in
        [(world_b.id, blocked, false), (world_a.id, open, true)]
    {
        let (release, held) = hold_fetch_lock(&locks, (holder_world, image_url.clone())).await;
        let inline = tokio::time::timeout(
            window,
            resolve_inline_image(deps(), world_b.id, image_url.clone(), String::new()),
        )
        .await;
        assert_eq!(
            inline.is_ok(),
            completes,
            "inline image, holder {holder_world}"
        );
        let thumbnail = tokio::time::timeout(
            window,
            resolve_thumbnail_asset(deps(), world_b.id, image_url.clone()),
        )
        .await;
        assert_eq!(
            thumbnail.is_ok(),
            completes,
            "thumbnail, holder {holder_world}"
        );
        let preview = tokio::time::timeout(
            window,
            resolve_preview_image(
                deps(),
                world_b.id,
                vec![image_url.clone()],
                image_url.clone(),
                String::new(),
                String::new(),
                image_url.clone(),
            ),
        )
        .await;
        assert_eq!(
            preview.is_ok(),
            completes,
            "preview image, holder {holder_world}"
        );
        let _ = release.send(());
        held.await.unwrap();
    }
}
