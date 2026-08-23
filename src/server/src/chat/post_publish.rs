//! Post-publish background enrichment: image fetches deferred until AFTER a
//! message's synchronous send/edit already returned and broadcast -- never
//! on the request path. The one entry point here re-publishes via
//! `WriteOrigin::ServerMessageRevision`, the SAME chokepoint
//! `handle_edit_message`/`handle_delete_message` use for their own message
//! revisions (this is the third caller of that origin, after edit and
//! delete).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::link_preview::fetch_image_bytes;
use super::{MessageEngine, Segment, MESSAGE_DOC_TYPE};
use crate::data::asset::{create_asset_from_bytes, NewAssetBytes};
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::Room;

/// One post-publish background job queued by `link_preview::enrich` for the
/// caller to run AFTER `Room::publish`'s synchronous send/edit already
/// returned. Never constructed before the message is durably stored -- a
/// fetch failure here must never block or delay the send/edit it enriches.
#[derive(Debug, Clone)]
pub enum PendingEnrichment {
    /// A `Segment::LinkPreview` (matched by its stored `url`, not an array
    /// index -- this task's own OCC re-read may observe a `content` array a
    /// concurrent edit reordered) whose synchronous scrape found an image
    /// candidate not yet fetched.
    PreviewImage {
        /// The `Segment::LinkPreview.url` this job targets.
        preview_url: String,
        /// The extracted (not yet fetched) image URL.
        image_url: String,
    },
}

/// What one resolved `PendingEnrichment` does to a message's stored `content`.
enum ResolvedEnrichment {
    /// Patch an existing `Segment::LinkPreview` (matched by `url`) with a
    /// resolved image asset id.
    ImageForPreview {
        /// The `Segment::LinkPreview.url` to match.
        preview_url: String,
        /// The resolved asset id.
        asset_id: Uuid,
    },
}

/// Grouped dependencies for `run_pending_enrichments` -- grouped instead of
/// five positional parameters (bringing the call to eight total, once
/// `write_barrier` joined `room`/`repo`/`client`/`assets_root`) to stay under
/// `clippy::too_many_arguments` by restructuring the signature, never by
/// suppressing the lint (same pattern as `data::asset::NewAssetBytes`).
pub struct PostPublishDeps {
    /// The world's room, republished into on the eventual `Operation::Update`.
    pub room: Arc<Room>,
    /// The repository, for both the OCC re-read and the eventual publish.
    pub repo: Arc<SqliteRepository>,
    /// The SSRF-guarded fetch client (the same one `enrich`'s synchronous
    /// scrape uses).
    pub client: Arc<reqwest::Client>,
    /// The asset-storage root a resolved image is committed under.
    pub assets_root: std::path::PathBuf,
    /// The backup write-quiesce barrier -- held read-side around every asset
    /// commit this pipeline performs, same as `http::assets::upload`/
    /// `replace`/`delete`.
    pub write_barrier: Arc<tokio::sync::RwLock<()>>,
}

/// Runs every queued `PendingEnrichment` for `message_id` concurrently, then
/// issues AT MOST ONE `WriteOrigin::ServerMessageRevision` `Operation::Update`
/// re-publishing whichever fields the jobs resolved -- never zero-to-many
/// separate republishes for one message's worth of pending work. Re-reads the
/// CURRENT stored document immediately before publishing (OCC pre-image),
/// exactly like `handle_edit_message`'s own re-read: the message may have
/// been edited or deleted by the time this task's fetches complete, and a
/// stale `old` value fails the `Update` closed rather than clobbering a
/// newer revision. A tombstoned or vanished message is a silent no-op.
pub async fn run_pending_enrichments(
    deps: PostPublishDeps,
    message_id: Uuid,
    world_id: Uuid,
    jobs: Vec<PendingEnrichment>,
) {
    if jobs.is_empty() {
        return;
    }
    let PostPublishDeps {
        room,
        repo,
        client,
        assets_root,
        write_barrier,
    } = deps;
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let repo = repo.clone();
        let client = client.clone();
        let assets_root = assets_root.clone();
        let write_barrier = write_barrier.clone();
        set.spawn(async move {
            resolve_job(repo, client, assets_root, write_barrier, world_id, job).await
        });
    }
    let mut resolved: Vec<ResolvedEnrichment> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(Some(r)) = joined {
            resolved.push(r);
        }
    }
    if resolved.is_empty() {
        return;
    }

    let Ok(Some(cur)) = repo.get_document(message_id).await else {
        return;
    };
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return;
    }
    let Ok(mut sys) =
        serde_json::from_value::<MessageEngine>(cur.engine.clone().unwrap_or_default())
    else {
        return;
    };
    if sys.deleted_at.is_some() {
        return; // a tombstoned message has no content left to enrich
    }
    let mut changed = false;
    for r in resolved {
        match r {
            ResolvedEnrichment::ImageForPreview {
                preview_url,
                asset_id,
            } => {
                for seg in sys.content.iter_mut() {
                    if let Segment::LinkPreview {
                        url,
                        image_asset_id,
                        ..
                    } = seg
                    {
                        if *url == preview_url {
                            *image_asset_id = Some(asset_id);
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    if !changed {
        return;
    }
    let Ok(new_engine) = serde_json::to_value(&sys) else {
        return;
    };
    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
        }],
    };
    // Attributed to the message's own sender, re-resolved live (`WorldRole`
    // is not consulted by `apply_intent`'s `ServerMessageRevision` branch --
    // see that chokepoint's doc -- so a departed member's default `Player`
    // role here affects only `Command.author` bookkeeping, nothing
    // authorization-relevant).
    let world_role = repo
        .member_role(world_id, sys.user_owner)
        .await
        .ok()
        .flatten()
        .unwrap_or(WorldRole::Player);
    let ctx = PermissionContext {
        user_id: sys.user_owner,
        world_role,
    };
    let now = crate::ws::time::now_millis();
    let _ = room
        .publish(
            repo.as_ref(),
            &ctx,
            vec![op],
            now,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
}

/// Dispatches one job to its resolver. `None` on any failure (network,
/// decode, asset creation) -- a failed background enrichment degrades
/// silently, exactly like the synchronous preview fetch it extends; there is
/// no error surface back to the sender for a job running long after their
/// own request already succeeded.
async fn resolve_job(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    write_barrier: Arc<tokio::sync::RwLock<()>>,
    world_id: Uuid,
    job: PendingEnrichment,
) -> Option<ResolvedEnrichment> {
    match job {
        PendingEnrichment::PreviewImage {
            preview_url,
            image_url,
        } => {
            resolve_preview_image(
                repo,
                client,
                assets_root,
                write_barrier,
                world_id,
                preview_url,
                image_url,
            )
            .await
        }
    }
}

/// De-dup-then-fetch for one `PreviewImage` job: checks the persisted
/// `link_preview_cache` row for `preview_url` FIRST and reuses an existing
/// `image_asset_id` verbatim on a hit -- never re-fetching or re-creating an
/// asset for a link this or any other message already imaged. On a miss,
/// fetches `image_url` through the SAME SSRF-guarded client `link_preview.rs`
/// already built (`fetch_image_bytes`), asset-ifies it via
/// `create_asset_from_bytes` (`created_by: None` -- no real user account
/// backs a server-fetched image, the same generalization
/// `Asset.created_by`'s own doc comment already covers), and records the
/// result for future hits. Holds `write_barrier`'s read side around the
/// asset commit, same as every other asset writer (`http::assets::upload`/
/// `replace`/`delete`) -- this is the first asset commit reachable from
/// outside a direct HTTP request, so it must join that same exclusion or an
/// in-server backup's file-copy could race a half-committed asset.
async fn resolve_preview_image(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    write_barrier: Arc<tokio::sync::RwLock<()>>,
    world_id: Uuid,
    preview_url: String,
    image_url: String,
) -> Option<ResolvedEnrichment> {
    if let Ok(Some(row)) = repo.get_link_preview_cache(&preview_url).await {
        if let Some(asset_id) = row.image_asset_id {
            return Some(ResolvedEnrichment::ImageForPreview {
                preview_url,
                asset_id,
            });
        }
    }
    let (content_type, bytes) = fetch_image_bytes(&client, &image_url, Duration::from_secs(5))
        .await
        .ok()?;
    let now = crate::ws::time::now_millis();
    let asset = {
        let _read_permit = write_barrier.read().await;
        create_asset_from_bytes(
            &repo,
            &assets_root,
            world_id,
            NewAssetBytes {
                bytes: &bytes,
                content_type: &content_type,
                original_name: "link-preview-image",
                created_by: None,
            },
            now,
        )
        .await
        .ok()?
    };
    let _ = repo
        .set_link_preview_cache_image(&preview_url, asset.id)
        .await;
    Some(ResolvedEnrichment::ImageForPreview {
        preview_url,
        asset_id: asset.id,
    })
}

#[cfg(test)]
mod tests {
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
            repo,
            unreachable_client,
            root.path().to_path_buf(),
            test_write_barrier(),
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
            repo.clone(),
            client,
            root.path().to_path_buf(),
            test_write_barrier(),
            world.id,
            preview_url.to_string(),
            image_url,
        )
        .await
        .expect("cache miss must fetch and create an asset");
        let ResolvedEnrichment::ImageForPreview { asset_id, .. } = resolved;
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
}
