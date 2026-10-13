//! This module runs image and oEmbed fetches after a message's send or edit has broadcast.
//!
//! The fetches run in the background AFTER the synchronous send/edit has returned, never on the
//! request path. `run_pending_enrichments` re-publishes under
//! `WriteOrigin::ServerMessageRevision`, the origin `handle_edit_message` and
//! `handle_delete_message` also use.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use uuid::Uuid;

use super::link_preview::{
    clean_text, fetch_image_bytes, fetch_json_bytes, MAX_IMAGE_BYTES, MAX_INLINE_IMAGE_BYTES,
    MAX_TITLE_CHARS,
};
use super::oembed::{OEmbedProvider, OEmbedResponse, OEmbedSegment};
use super::{MessageEngine, Segment, MESSAGE_DOC_TYPE};
use crate::data::asset::{create_asset_from_bytes, NewAssetBytes};
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::Room;

/// Process-wide registry of per-URL fetch locks, keyed by the URL a persisted
/// `link_preview_cache` row is stored under. `resolve_preview_image` keys on the job's
/// `cache_urls` and `resolve_thumbnail_asset`/`resolve_inline_image` on the image URL, so two
/// concurrent post-publish jobs racing on a shared key run one after the other, and the second
/// reads the asset id the first recorded instead of creating an orphaned `Asset` row and file
/// (see `with_preview_url_locks` for the concurrency argument, including why an entry is removed
/// without a race). The map has the keyed-registry shape of `ws::room::RoomRegistry.rooms`. That
/// map holds one entry per world, while this one would gain an entry for every distinct
/// link-preview URL seen, so `release_preview_lock` removes an entry once its last holder
/// releases.
pub type PreviewFetchLocks = Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// Runs `f` while holding the lock of every distinct key in `keys`, get-or-inserting a fresh
/// `Mutex` per key on first use and removing each entry no other caller holds once `f` returns,
/// panics or the returned future is dropped.
///
/// # Lock order
///
/// Keys are acquired in ascending string order. Two callers whose key sets overlap therefore
/// acquire the shared keys in the same relative order, so neither can hold a key the other
/// awaits while awaiting a key the first holds.
///
/// # Cleanup-race reasoning
///
/// A version that drops the mutex guards and then separately checks whether to remove each map
/// entry has a TOCTOU gap: a new waiter can call `locks.entry(key)` between the guard drop and
/// the removal check, clone the `Arc` about to be removed, and wait on a `Mutex` the map is about
/// to discard, while a later caller's `or_insert_with` creates a second `Mutex` for the same key.
/// Two callers would then hold two different mutexes for one URL, defeating the registry.
///
/// This function never separates the strong-count check from the removal: both happen under the
/// shard lock of one `locks.entry(key)` call in `release_preview_lock`, made after the guards and
/// the caller's own `Arc` clone for that key are dropped. `DashMap::entry` blocks any other
/// `entry()`, `get()` or `insert()` for that key while the returned `Entry` is alive, and this
/// function is the only code that clones an `Arc` out of the map, so no concurrent caller can
/// clone it between the count read and the removal. Every `Arc` is cloned out of the map before
/// any lock is awaited, so a holder other than this caller makes the count exceed 1 for as long
/// as it holds or awaits the key.
///
/// `HeldPreviewLocks` runs that release from `Drop`, so a caller whose future is dropped while
/// awaiting a lock or inside `f` (an aborted task) releases the same way a completed one does.
async fn with_preview_url_locks<F, Fut, T>(locks: &PreviewFetchLocks, keys: &[String], f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let mut sorted: Vec<&String> = keys.iter().collect();
    sorted.sort();
    sorted.dedup();
    let entries = sorted
        .into_iter()
        .map(|key| {
            let arc = locks
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone();
            (key.clone(), arc)
        })
        .collect::<Vec<_>>();
    let mut held = HeldPreviewLocks {
        locks,
        guards: Vec::with_capacity(entries.len()),
        entries,
    };
    for index in 0..held.entries.len() {
        let guard = held.entries[index].1.clone().lock_owned().await;
        held.guards.push(guard);
    }
    f().await
}

/// The registry entries one `with_preview_url_locks` call cloned out of the map and the guards it
/// has acquired so far, released together when it is dropped.
struct HeldPreviewLocks<'a> {
    /// The registry the entries were cloned from.
    locks: &'a PreviewFetchLocks,
    /// Each distinct key with this caller's clone of its map entry, in ascending key order.
    entries: Vec<(String, Arc<tokio::sync::Mutex<()>>)>,
    /// The guards acquired so far, in `entries` order.
    guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for HeldPreviewLocks<'_> {
    /// Drops the guards first, because each guard owns an `Arc` clone that `release_preview_lock`'s
    /// strong-count check would otherwise count as another holder.
    fn drop(&mut self) {
        self.guards.clear();
        for (key, arc) in self.entries.drain(..) {
            release_preview_lock(self.locks, key, arc);
        }
    }
}

/// Drops the caller's `arc` for `key` and removes the map entry when the map holds the only
/// remaining clone.
///
/// Dropping before the count check makes the last of several concurrent holders to release see
/// a count of 1, whichever order they release in. Counting while still holding `arc` would leave
/// two holders that each release while the other's clone is alive both seeing a higher count,
/// and the entry would never be removed.
fn release_preview_lock(locks: &PreviewFetchLocks, key: String, arc: Arc<tokio::sync::Mutex<()>>) {
    drop(arc);
    if let Entry::Occupied(occ) = locks.entry(key) {
        if Arc::strong_count(occ.get()) == 1 {
            occ.remove();
        }
    }
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
    /// candidate not yet fetched. One job serves the message's candidate URLs whose previews
    /// ended on the same `preview_url`; the `image_url` fetched is the first such preview's in
    /// message order that carries one.
    PreviewImage {
        /// The candidate URLs `enrich` extracted from the message, in message order, whose
        /// previews carry this job's `preview_url`: the keys of the persisted
        /// `link_preview_cache` rows the job reads and writes. `enrich` builds it with at least
        /// one entry.
        cache_urls: Vec<String>,
        /// The `url` of the `Segment::LinkPreview` this job patches: the address the fetch
        /// ended on (normalized by `Url::to_string`, post-redirect). Differs from a
        /// `cache_urls` entry whenever the fetch's final address differs from the extracted
        /// href.
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
        /// The candidate URL `enrich` extracted from the message.
        post_url: String,
        /// The allowlisted provider this URL matched.
        provider: OEmbedProvider,
    },
    /// A Markdown/HTML image source `sanitize` collected into
    /// `Sanitized.image_urls` (already gated on `policy.images()` -- see
    /// `link_preview::enrich`'s doc). Resolves to a brand-new
    /// `Segment::Image`, never patching an existing segment.
    InlineImage {
        /// The image URL to fetch, exactly as `sanitize` recorded it.
        image_url: String,
        /// The image's alt text, stored verbatim on the resulting
        /// `Segment::Image`.
        alt: String,
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
    /// Append a brand-new `Segment::Image` to `content` -- never patches an
    /// existing segment (see `PendingEnrichment::InlineImage`'s doc).
    NewImageSegment(Segment),
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
    /// `Config.retain_originals`, forwarded to every asset commit.
    pub retain_originals: bool,
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
        retain_originals,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let deps = FetchDeps {
            repo: repo.clone(),
            client: client.clone(),
            assets_root: assets_root.clone(),
            retain_originals,
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
            ResolvedEnrichment::NewImageSegment(segment) => {
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
    /// `Config.retain_originals`, forwarded to every asset commit.
    retain_originals: bool,
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
            cache_urls,
            preview_url,
            image_url,
        } => resolve_preview_image(deps, world_id, cache_urls, preview_url, image_url).await,
        PendingEnrichment::OEmbed { post_url, provider } => {
            resolve_oembed(deps, world_id, post_url, provider).await
        }
        PendingEnrichment::InlineImage { image_url, alt } => {
            resolve_inline_image(deps, world_id, image_url, alt).await
        }
    }
}

/// De-dup-then-fetch for one `PreviewImage` job: reads the persisted `link_preview_cache` row of
/// every `cache_urls` entry FIRST and, when any row carries an `image_asset_id`, reuses it
/// verbatim, writing it onto the rows that lack one -- no refetch and no second asset for a link
/// this or another message already imaged. On a miss, fetches `image_url` through the SAME
/// SSRF-guarded client `enrich` uses (`link_preview::fetch_image_bytes`), asset-ifies it via
/// `create_asset_from_bytes` (`created_by: None`: no user account backs a server-fetched image),
/// and records the asset id on the `cache_urls` rows that exist and hold no id, so the patched
/// segment carries the job's asset even when a write that does not take the lock recorded another
/// id on a row first. Holds `write_barrier`'s read side around the asset commit, as
/// `http::assets::upload`/`replace`/`delete` do: this commit runs from the post-publish job
/// rather than an HTTP handler, so it joins the same exclusion or an in-server backup's file copy
/// could race a half-committed asset. Runs the whole read-fetch-write sequence under the `preview_fetch_locks` locks of all
/// `cache_urls` (see `with_preview_url_locks`), so when the first of two concurrent jobs sharing
/// a `cache_urls` entry succeeds, the second finds its recorded asset id instead of creating
/// another asset.
async fn resolve_preview_image(
    deps: FetchDeps,
    world_id: Uuid,
    cache_urls: Vec<String>,
    preview_url: String,
    image_url: String,
) -> Option<ResolvedEnrichment> {
    let FetchDeps {
        repo,
        client,
        assets_root,
        retain_originals,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    // The closure moves `cache_urls`, so the lock keys are a separate clone.
    let lock_keys = cache_urls.clone();
    with_preview_url_locks(&preview_fetch_locks, &lock_keys, move || async move {
        let mut known: Option<Uuid> = None;
        let mut unset: Vec<&String> = Vec::new();
        for cache_url in &cache_urls {
            match repo.get_link_preview_cache(cache_url).await {
                Ok(Some(row)) => match row.image_asset_id {
                    Some(id) => {
                        known.get_or_insert(id);
                    }
                    None => unset.push(cache_url),
                },
                _ => unset.push(cache_url),
            }
        }
        if let Some(asset_id) = known {
            record_preview_image(&repo, unset, asset_id).await;
            return Some(ResolvedEnrichment::ImageForPreview {
                preview_url,
                asset_id,
            });
        }
        let (content_type, bytes) =
            fetch_image_bytes(&client, &image_url, Duration::from_secs(5), MAX_IMAGE_BYTES)
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
                    provenance: crate::data::asset::Provenance::LinkPreview,
                    retain_originals,
                },
                now,
            )
            .await
            .ok()?
        };
        record_preview_image(&repo, cache_urls.iter().collect(), asset.id).await;
        Some(ResolvedEnrichment::ImageForPreview {
            preview_url,
            asset_id: asset.id,
        })
    })
    .await
}

/// Records `asset_id` on the `link_preview_cache` row of each of `cache_urls` that has no asset
/// id yet (`set_link_preview_cache_image` leaves an existing id); a failed write is logged and the
/// remaining rows are still written.
async fn record_preview_image(repo: &SqliteRepository, cache_urls: Vec<&String>, asset_id: Uuid) {
    for cache_url in cache_urls {
        if let Err(e) = repo.set_link_preview_cache_image(cache_url, asset_id).await {
            tracing::warn!(?e, %cache_url, "link-preview image asset persist failed");
        }
    }
}

/// De-dup-then-fetch for one `InlineImage` job: checks the persisted
/// `link_preview_cache` row for `image_url` FIRST (same table, keyed here by
/// the chat image's own URL rather than a previewed page's URL) and reuses an
/// existing `image_asset_id` verbatim on a hit -- never re-fetching or
/// re-creating an asset for a URL some other message already imaged. On a
/// miss, fetches `image_url` through the SAME SSRF-guarded client
/// (`fetch_image_bytes`, capped at `MAX_INLINE_IMAGE_BYTES` -- larger than
/// `resolve_preview_image`'s `MAX_IMAGE_BYTES` since this is full-size
/// message content, not a thumbnail), asset-ifies it via
/// `create_asset_from_bytes` with `Provenance::ChatImage` (`created_by: None`,
/// same generalization as `resolve_preview_image`), and records the result
/// for future hits. Holds `write_barrier`'s read side around the asset
/// commit and runs the whole check-then-fetch-then-set sequence under
/// `preview_fetch_locks`' per-`image_url` lock, identically to
/// `resolve_preview_image` -- see that function's doc for the full
/// concurrency argument.
async fn resolve_inline_image(
    deps: FetchDeps,
    world_id: Uuid,
    image_url: String,
    alt: String,
) -> Option<ResolvedEnrichment> {
    // `MAX_IMAGE_ALT_CHARS` is a REFUSAL cap (see its own doc), never a
    // truncation -- a Markdown/HTML `alt` this long has no ingest-time
    // rejection point (unlike a `[[asset:...]]` span's `AltTooLong`), so this
    // background job drops silently, same posture as every other failure
    // this pipeline already degrades on (a blocked host, a fetch error).
    if alt.chars().count() > super::MAX_IMAGE_ALT_CHARS {
        return None;
    }
    let FetchDeps {
        repo,
        client,
        assets_root,
        retain_originals,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    let lock_keys = [image_url.clone()];
    with_preview_url_locks(&preview_fetch_locks, &lock_keys, move || async move {
        if let Ok(Some(row)) = repo.get_link_preview_cache(&image_url).await {
            if let Some(asset_id) = row.image_asset_id {
                return Some(ResolvedEnrichment::NewImageSegment(Segment::Image {
                    asset_id,
                    alt,
                }));
            }
        }
        let (content_type, bytes) = fetch_image_bytes(
            &client,
            &image_url,
            Duration::from_secs(5),
            MAX_INLINE_IMAGE_BYTES,
        )
        .await
        .ok()?;
        let now = crate::ws::time::now_millis();
        // The URL's own last path segment, when present and non-empty, gives
        // a more useful stored file name than a fixed literal (unlike
        // `resolve_preview_image`/`resolve_thumbnail_asset`'s thumbnails,
        // an inline chat image is the message's own primary content and may
        // later be browsed/downloaded through the asset library by that
        // name).
        let original_name = url::Url::parse(&image_url)
            .ok()
            .and_then(|u| {
                u.path_segments()
                    .and_then(|mut segs| segs.next_back().map(str::to_string))
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "image".to_string());
        let asset = {
            let _read_permit = write_barrier.read().await;
            create_asset_from_bytes(
                &repo,
                &assets_root,
                world_id,
                NewAssetBytes {
                    bytes: &bytes,
                    content_type: &content_type,
                    original_name: &original_name,
                    created_by: None,
                    provenance: crate::data::asset::Provenance::ChatImage,
                    retain_originals,
                },
                now,
            )
            .await
            .ok()?
        };
        // No prior `upsert_link_preview_cache` call exists for `image_url`
        // (unlike `resolve_preview_image`'s target, which the synchronous
        // href-preview scrape already upserted) -- an inline chat image URL
        // never went through that stage, so a row must be created here
        // before `set_link_preview_cache_image` can attach an asset id to
        // it. Same two-call shape as `resolve_thumbnail_asset`.
        let _ = repo
            .upsert_link_preview_cache(&image_url, None, None, now)
            .await;
        let _ = repo
            .set_link_preview_cache_image(&image_url, asset.id)
            .await;
        Some(ResolvedEnrichment::NewImageSegment(Segment::Image {
            asset_id: asset.id,
            alt,
        }))
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
        retain_originals,
        write_barrier,
        preview_fetch_locks,
    } = deps;
    // Same per-URL exclusion + re-check-after-lock treatment as
    // `resolve_preview_image`, keyed by the thumbnail's own URL (the same key
    // space `resolve_preview_image` uses -- a page previewed AND oEmbed-
    // thumbnailed via the identical image URL serializes against itself
    // too).
    let lock_keys = [thumbnail_url.clone()];
    with_preview_url_locks(&preview_fetch_locks, &lock_keys, move || async move {
        if let Ok(Some(row)) = repo.get_link_preview_cache(&thumbnail_url).await {
            if let Some(asset_id) = row.image_asset_id {
                return Some(asset_id);
            }
        }
        let (content_type, bytes) = fetch_image_bytes(
            &client,
            &thumbnail_url,
            Duration::from_secs(5),
            MAX_IMAGE_BYTES,
        )
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
                    provenance: crate::data::asset::Provenance::LinkPreview,
                    retain_originals,
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
mod tests;
