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
        write_barrier: test_write_barrier(),
        preview_fetch_locks,
    }
}

/// A publish helper: sends a plain message and returns its message id --
/// no `og:image` involved (these tests exercise `run_pending_enrichments`
/// directly, not the synchronous scrape).
async fn seed_message(room: &Room, repo: &SqliteRepository, ctx: &PermissionContext) -> Uuid {
    let rate = PingRateLimiter::new();
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room,
            repo,
            ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &build_link_preview_client(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
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
    };
    repo.insert_asset(&asset).await.unwrap();
    let preview_url = "https://cached.example/";
    repo.upsert_link_preview_cache(preview_url, Some("T"), Some("D"), 0)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(preview_url, asset.id)
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
        preview_url.to_string(),
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
    repo.upsert_link_preview_cache(preview_url, Some("T"), Some("D"), 0)
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
        preview_url.to_string(),
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
        .get_link_preview_cache(preview_url)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.image_asset_id, Some(asset_id));
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
        preview_url: "https://match.example/".into(),
        image_url: "https://match.example/og.png".into(),
    }];
    // Pre-seed the persisted cache row so resolution never touches the
    // network -- this test targets the patch-by-url logic, not the fetch.
    repo.upsert_link_preview_cache("https://match.example/", None, None, 0)
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
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image("https://match.example/", asset_id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
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
        preview_url: "https://match.example/".into(),
        image_url: "https://match.example/og.png".into(),
    }];
    repo.upsert_link_preview_cache("https://match.example/", None, None, 0)
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
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image("https://match.example/", asset.id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
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
        preview_url: "https://match.example/".into(),
        image_url: "https://match.example/og.png".into(),
    }];
    repo.upsert_link_preview_cache("https://match.example/", None, None, 0)
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
    };
    repo.insert_asset(&asset).await.unwrap();
    repo.set_link_preview_cache_image("https://match.example/", asset_id)
        .await
        .unwrap();

    run_pending_enrichments(
        PostPublishDeps {
            room: room.clone(),
            repo: repo.clone(),
            client: Arc::new(build_link_preview_client()),
            assets_root: std::path::PathBuf::new(),
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
    repo.upsert_link_preview_cache(&preview_url, Some("T"), Some("D"), 0)
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
                preview_url,
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
                preview_url,
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
    // resolvers have completed -- see `with_preview_url_lock`'s cleanup
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
        repo.upsert_link_preview_cache(&preview_url, Some("T"), Some("D"), 0)
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
            preview_url,
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
