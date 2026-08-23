//! Post-publish background enrichment: image fetches deferred until AFTER a
//! message's synchronous send/edit already returned and broadcast -- never
//! on the request path. The one entry point here re-publishes via
//! `WriteOrigin::ServerMessageRevision`, the SAME chokepoint
//! `handle_edit_message`/`handle_delete_message` use for their own message
//! revisions (this is the third caller of that origin, after edit and
//! delete).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use uuid::Uuid;

use super::link_preview::{clean_text, fetch_image_bytes, fetch_json_bytes, MAX_TITLE_CHARS};
use super::oembed::{OEmbedProvider, OEmbedResponse, OEmbedSegment};
use super::{MessageEngine, Segment, MESSAGE_DOC_TYPE};
use crate::data::asset::{create_asset_from_bytes, NewAssetBytes};
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::Room;

/// Process-wide registry of per-URL fetch locks: `resolve_preview_image`/
/// `resolve_thumbnail_asset` key on the target URL (the `preview_url`/
/// `thumbnail_url` each already keys the persisted `link_preview_cache` table
/// by) so two concurrent post-publish jobs racing to resolve the identical
/// URL serialize into one fetch+asset-create instead of each creating its own
/// orphaned `Asset` row/file for the same link (see `with_preview_url_lock`'s
/// doc for the full concurrency argument, including why the entry is removed
/// without a race). Same `DashMap`-keyed-registry shape as
/// `ws::room::RoomRegistry.rooms` -- unlike that map (bounded by world
/// count), distinct link-preview URLs are unbounded over a long-running
/// server's lifetime, so THIS map's entries must be reclaimed once no
/// concurrent caller still needs them.
pub type PreviewFetchLocks = Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// Runs `f` while holding the per-`key` lock from `locks`, get-or-inserting a
/// fresh `Mutex` on first use and reclaiming the map entry once no concurrent
/// caller still references it.
///
/// # Cleanup-race reasoning
///
/// The naive version of this (drop the mutex guard, THEN separately check
/// whether to remove the map entry) has a TOCTOU gap: a new waiter can call
/// `locks.entry(key)` between this task's guard-drop and its own removal
/// check, clone the Arc that is about to be removed, and start waiting on a
/// `Mutex` instance the map is seconds away from discarding -- while a LATER
/// caller's `or_insert_with` then manufactures a SECOND, different `Mutex`
/// instance for the same key, so two callers hold two different mutexes for
/// one URL and the whole point of this registry (mutual exclusion per URL) is
/// defeated.
///
/// This avoids that gap by never separating the strong-count check from the
/// removal: both happen while holding the SAME `DashMap` shard lock, via one
/// continuous `locks.entry(key)` call after the mutex guard is already
/// dropped. `DashMap::entry` blocks any other `entry()`/`get()`/`insert()`
/// call for the same key for as long as the returned `Entry` is alive, so no
/// concurrent caller can observe or clone the `Arc` between the strong-count
/// read and the removal. `Arc::strong_count(occ.get()) <= 2` means only the
/// map's own stored clone (1) plus this function's local `arc` binding (1,
/// not yet dropped) remain -- nobody else cloned it while we were not holding
/// the shard lock, so it is safe to remove. A strong count above 2 means a
/// concurrent waiter already cloned the same `Arc` (either mid-wait on the
/// mutex, or about to call `.lock().await` on it), so the entry is left in
/// place for them to keep using -- removing it here would be the exact bug
/// this reasoning exists to prevent, just triggered from the opposite
/// direction.
async fn with_preview_url_lock<F, Fut, T>(locks: &PreviewFetchLocks, key: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let arc = locks
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let result = {
        let _guard = arc.lock().await;
        f().await
    };
    if let Entry::Occupied(occ) = locks.entry(key.to_string()) {
        if Arc::strong_count(occ.get()) <= 2 {
            occ.remove();
        }
    }
    result
}

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
    /// An allowlisted-host URL (see `chat::oembed`) matched during href
    /// collection at `enrich` time -- queries `provider`'s oEmbed endpoint
    /// and appends a brand-new `Segment::OEmbed`, never patching an
    /// existing segment (unlike `PreviewImage`, no `Segment::LinkPreview`
    /// was ever appended for this URL -- see `link_preview::enrich`'s
    /// mutually-exclusive routing).
    OEmbed {
        /// The URL as posted in the message.
        post_url: String,
        /// The allowlisted provider this URL matched.
        provider: OEmbedProvider,
    },
}

/// What one resolved `PendingEnrichment` does to a message's stored `content`.
#[derive(Debug)]
enum ResolvedEnrichment {
    /// Patch an existing `Segment::LinkPreview` (matched by `url`) with a
    /// resolved image asset id.
    ImageForPreview {
        /// The `Segment::LinkPreview.url` to match.
        preview_url: String,
        /// The resolved asset id.
        asset_id: Uuid,
    },
    /// Append a brand-new `Segment::OEmbed` to `content` -- never patches an
    /// existing segment (see `PendingEnrichment::OEmbed`'s doc).
    NewOEmbedSegment(Segment),
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
    /// Per-URL fetch-lock registry serializing concurrent
    /// `resolve_preview_image`/`resolve_thumbnail_asset` jobs targeting the
    /// identical URL -- see `PreviewFetchLocks`'s doc.
    pub preview_fetch_locks: PreviewFetchLocks,
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
        preview_fetch_locks,
    } = deps;
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let deps = FetchDeps {
            repo: repo.clone(),
            client: client.clone(),
            assets_root: assets_root.clone(),
            write_barrier: write_barrier.clone(),
            preview_fetch_locks: preview_fetch_locks.clone(),
        };
        set.spawn(async move { resolve_job(deps, world_id, job).await });
    }
    let mut resolved: Vec<ResolvedEnrichment> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(Some(r)) = joined {
            resolved.push(r);
        }
    }
    publish_resolved(room, repo, message_id, world_id, resolved).await;
}

/// Applies every already-resolved `ResolvedEnrichment` to `message_id`'s
/// stored `content` and issues AT MOST ONE `WriteOrigin::ServerMessageRevision`
/// `Operation::Update` re-publishing the result -- factored out of
/// `run_pending_enrichments` so a test can exercise this OCC-read/patch/
/// publish tail directly against a `resolved` list built from constituent
/// pieces (`fetch_json_bytes`/`resolve_thumbnail_asset`), without needing a
/// live fetch against `OEmbedProvider::endpoint`'s fixed real provider host.
/// A no-op on an empty `resolved` list.
async fn publish_resolved(
    room: Arc<Room>,
    repo: Arc<SqliteRepository>,
    message_id: Uuid,
    world_id: Uuid,
    resolved: Vec<ResolvedEnrichment>,
) {
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
            ResolvedEnrichment::NewOEmbedSegment(segment) => {
                sys.content.push(segment);
                changed = true;
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

/// Fetch/asset-commit dependencies shared by every resolver
/// (`resolve_preview_image`/`resolve_oembed`/`resolve_thumbnail_asset`) --
/// grouped the same way `PostPublishDeps` groups its own five fields, to stay
/// under `clippy::too_many_arguments` by restructuring the signature, never
/// by suppressing the lint.
#[derive(Clone)]
struct FetchDeps {
    /// The repository, for the persisted `link_preview_cache` read/write and
    /// the eventual asset commit.
    repo: Arc<SqliteRepository>,
    /// The SSRF-guarded fetch client (the same one `enrich`'s synchronous
    /// scrape uses).
    client: Arc<reqwest::Client>,
    /// The asset-storage root a resolved image is committed under.
    assets_root: std::path::PathBuf,
    /// The backup write-quiesce barrier -- held read-side around every asset
    /// commit, same as `PostPublishDeps.write_barrier`.
    write_barrier: Arc<tokio::sync::RwLock<()>>,
    /// Per-URL fetch-lock registry -- see `PreviewFetchLocks`'s doc.
    preview_fetch_locks: PreviewFetchLocks,
}

/// Dispatches one job to its resolver. `None` on any failure (network,
/// decode, asset creation) -- a failed background enrichment degrades
/// silently, exactly like the synchronous preview fetch it extends; there is
/// no error surface back to the sender for a job running long after their
/// own request already succeeded.
async fn resolve_job(
    deps: FetchDeps,
    world_id: Uuid,
    job: PendingEnrichment,
) -> Option<ResolvedEnrichment> {
    match job {
        PendingEnrichment::PreviewImage {
            preview_url,
            image_url,
        } => resolve_preview_image(deps, world_id, preview_url, image_url).await,
        PendingEnrichment::OEmbed { post_url, provider } => {
            resolve_oembed(deps, world_id, post_url, provider).await
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
/// in-server backup's file-copy could race a half-committed asset. Runs the
/// WHOLE check-then-fetch-then-set sequence under `preview_fetch_locks`'
/// per-`preview_url` lock (see `with_preview_url_lock`) so two concurrent
/// jobs for the identical URL never both observe a cache miss and each
/// create their own orphaned `Asset`.
async fn resolve_preview_image(
    deps: FetchDeps,
    world_id: Uuid,
    preview_url: String,
    image_url: String,
) -> Option<ResolvedEnrichment> {
    let FetchDeps {
        repo,
        client,
        assets_root,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    // Locks for the ENTIRE check-then-fetch-then-set sequence -- a concurrent
    // waiter for the identical `preview_url` must observe a persisted-cache
    // HIT here and skip the fetch entirely, never just serialize into a
    // second redundant fetch+asset-create for the same link. Keyed on a
    // clone (not `&preview_url`) so the closure below can move the original
    // `preview_url` into its `ResolvedEnrichment` without a borrow conflict.
    let lock_key = preview_url.clone();
    with_preview_url_lock(&preview_fetch_locks, &lock_key, move || async move {
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
    })
    .await
}

/// Queries `provider`'s oEmbed endpoint for `post_url` (see
/// `OEmbedProvider::endpoint` -- the endpoint HOST is always the fixed
/// allowlisted host, `post_url` only ever contributes the `url` query
/// value), deserializes the response into `OEmbedResponse` (structurally
/// incapable of carrying a provider's `html` field through -- see that
/// type's doc), resolves a thumbnail asset if one is present, and builds a
/// brand-new `Segment::OEmbed` for `run_pending_enrichments` to append. `None`
/// on any failure (network, decode) -- a failed background oEmbed fetch
/// degrades silently, exactly like `resolve_preview_image`.
async fn resolve_oembed(
    deps: FetchDeps,
    world_id: Uuid,
    post_url: String,
    provider: OEmbedProvider,
) -> Option<ResolvedEnrichment> {
    let endpoint = provider.endpoint(&post_url)?;
    let body = fetch_json_bytes(&deps.client, &endpoint, Duration::from_secs(5))
        .await
        .ok()?;
    let parsed: OEmbedResponse = serde_json::from_slice(&body).ok()?;
    let thumbnail_asset_id = match parsed.thumbnail_url {
        Some(thumbnail_url) => resolve_thumbnail_asset(deps, world_id, thumbnail_url).await,
        None => None,
    };
    // Capped the same way `link_preview::extract_preview`'s own scraped
    // title is: a provider's JSON is otherwise bounded only by the whole
    // response's `MAX_JSON_BYTES`, so an uncapped `title`/`author_name`
    // could inflate a stored message's `content` far past this codebase's
    // established title-length invariant.
    let title = parsed.title.map(|t| clean_text(&t, MAX_TITLE_CHARS));
    let author_name = parsed.author_name.map(|a| clean_text(&a, MAX_TITLE_CHARS));
    Some(ResolvedEnrichment::NewOEmbedSegment(Segment::OEmbed(
        OEmbedSegment {
            url: post_url,
            provider_name: provider.name().to_string(),
            title,
            author_name,
            thumbnail_asset_id,
        },
    )))
}

/// De-dup-then-fetch for one oEmbed thumbnail: checks the persisted
/// `link_preview_cache` row for `thumbnail_url` FIRST (the same table
/// `resolve_preview_image` uses, keyed here by the thumbnail's own URL
/// rather than the previewed page's URL) and reuses an existing
/// `image_asset_id` verbatim on a hit. On a miss, fetches `thumbnail_url`
/// through the SAME SSRF-guarded client (`fetch_image_bytes`), asset-ifies
/// it via `create_asset_from_bytes` (`created_by: None`, same
/// generalization as `resolve_preview_image`), and records the result for
/// future hits. Holds `write_barrier`'s read side around the asset commit,
/// same as `resolve_preview_image` -- this is a second asset-commit path
/// reachable from outside a direct HTTP request and must join the same
/// exclusion, or an in-server backup's file-copy could race a
/// half-committed asset.
async fn resolve_thumbnail_asset(
    deps: FetchDeps,
    world_id: Uuid,
    thumbnail_url: String,
) -> Option<Uuid> {
    let FetchDeps {
        repo,
        client,
        assets_root,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    // Same per-URL exclusion + re-check-after-lock treatment as
    // `resolve_preview_image`, keyed by the thumbnail's own URL (the same key
    // space `resolve_preview_image` uses -- a page previewed AND oEmbed-
    // thumbnailed via the identical image URL serializes against itself
    // too).
    let lock_key = thumbnail_url.clone();
    with_preview_url_lock(&preview_fetch_locks, &lock_key, move || async move {
        if let Ok(Some(row)) = repo.get_link_preview_cache(&thumbnail_url).await {
            if let Some(asset_id) = row.image_asset_id {
                return Some(asset_id);
            }
        }
        let (content_type, bytes) =
            fetch_image_bytes(&client, &thumbnail_url, Duration::from_secs(5))
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
                    original_name: "oembed-thumbnail",
                    created_by: None,
                },
                now,
            )
            .await
            .ok()?
        };
        let _ = repo
            .upsert_link_preview_cache(&thumbnail_url, None, None, now)
            .await;
        let _ = repo
            .set_link_preview_cache_image(&thumbnail_url, asset.id)
            .await;
        Some(asset.id)
    })
    .await
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
}
