//! GM-only `AudioTransport` handling: authorizes, resolves the one playlist an op could need,
//! applies the pure `state::apply`, and commits the `audio-state` Update under
//! `WriteOrigin::AudioTransport`. Also `on_active_scene`, the server rule that swaps a scene's
//! ambience playlist when `world-settings.activeScene` commits (`ws::room::Room::publish`'s own
//! hook calls it — never a client-driven path).

use std::collections::HashMap;
use std::fmt;

use uuid::Uuid;

use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::engine::{
    self as eng, AudioChannel, AudioStateEngine, PlaylistEngine, AUDIO_STATE_DOC_TYPE,
    PLAYLIST_DOC_TYPE,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::protocol::AudioOp;
use crate::ws::room::Room;

/// Refusal reasons `handle_transport` surfaces via `ServerMsg::AudioError.reason`
/// (player-presentable `Display`).
#[derive(Debug)]
pub(crate) enum TransportError {
    /// The caller is not a GM.
    Forbidden,
    /// This world has no `audio-state` document yet (should not arise once every world is
    /// seeded with one; defensive for a world that never re-seeded).
    NoAudioState,
    /// A repository read/write failed.
    Internal,
    /// The pure engine refused the op (see `audio::state::AudioError`).
    Op(crate::audio::state::AudioError),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Forbidden => f.write_str("forbidden"),
            TransportError::NoAudioState => f.write_str("this world has no audio state yet"),
            TransportError::Internal => f.write_str("internal error"),
            TransportError::Op(e) => write!(f, "{e}"),
        }
    }
}

/// The one playlist `op` could need, pre-fetched here (async) so the pure `state::apply` stays
/// I/O-free: `Play` names it directly; `Next`/`Prev` need the advancing entry's OWN `playlist`
/// field, read from `state` before the fetch.
async fn prefetch_playlist(
    repo: &dyn Repository,
    state: &AudioStateEngine,
    op: &AudioOp,
) -> HashMap<Uuid, PlaylistEngine> {
    let needed = match op {
        AudioOp::Play {
            playlist: Some(pid),
            ..
        } => Some(*pid),
        AudioOp::Next { id } | AudioOp::Prev { id } => state
            .playing
            .iter()
            .find(|t| t.id == *id)
            .and_then(|t| t.playlist),
        _ => None,
    };
    let mut cache = HashMap::new();
    if let Some(pid) = needed {
        if let Ok(Some(doc)) = repo.get_document(pid).await {
            if doc.doc_type == PLAYLIST_DOC_TYPE {
                if let Some(v) = &doc.engine {
                    if let Ok(pl) = serde_json::from_value::<PlaylistEngine>(v.clone()) {
                        cache.insert(pid, pl);
                    }
                }
            }
        }
    }
    cache
}

/// GM-only: applies `op` to the world's `audio-state` singleton and commits the Update under
/// `WriteOrigin::AudioTransport`. `AudioOp::Next`'s elapsed-duration gate lives HERE (not in the
/// pure `state::apply`) because it needs the asset row's `durationMs`: a report that arrives
/// before the current track's computed end time is a silent no-op (`Ok(())`, no `AudioError`),
/// which is what makes "the first client's report wins" true without punishing a merely-early
/// second reporter with a visible refusal.
pub(crate) async fn handle_transport(
    repo: &dyn Repository,
    ctx: &PermissionContext,
    room: &Room,
    world_id: Uuid,
    op: AudioOp,
    now: i64,
) -> Result<(), TransportError> {
    if ctx.world_role != WorldRole::Gm {
        return Err(TransportError::Forbidden);
    }
    let docs = repo
        .query_documents(world_id, AUDIO_STATE_DOC_TYPE)
        .await
        .map_err(|_| TransportError::Internal)?;
    let doc = docs
        .into_iter()
        .next()
        .ok_or(TransportError::NoAudioState)?;
    let state: AudioStateEngine = eng::engine_of(&doc);
    let now_f = now as f64;

    if let AudioOp::Next { id } = &op {
        let Some(track) = state.playing.iter().find(|t| t.id == *id) else {
            return Ok(()); // stale id: already advanced by an earlier report
        };
        if let Ok(asset_id) = Uuid::parse_str(&track.asset) {
            if let Ok(Some(asset)) = repo.get_asset(asset_id).await {
                if let Some(duration_ms) = asset.meta.duration_ms {
                    if now_f < track.started_at + duration_ms as f64 {
                        return Ok(()); // premature report
                    }
                }
            }
        }
    }

    let cache = prefetch_playlist(repo, &state, &op).await;
    let lookup = |id: Uuid| cache.get(&id).cloned();
    let next =
        crate::audio::state::apply(&state, &op, now_f, &lookup).map_err(TransportError::Op)?;
    commit_audio_state(repo, ctx, room, &doc, &state, &next, now).await
}

/// Shared commit tail: writes `next` over `current` at `/engine` on `doc`, under
/// `WriteOrigin::AudioTransport`, IFF the two differ (an unchanged state — e.g. `on_active_scene`
/// finding nothing to swap — commits nothing).
async fn commit_audio_state(
    repo: &dyn Repository,
    ctx: &PermissionContext,
    room: &Room,
    doc: &crate::data::document::Document,
    current: &AudioStateEngine,
    next: &AudioStateEngine,
    now: i64,
) -> Result<(), TransportError> {
    if next == current {
        return Ok(());
    }
    let change = FieldChange {
        path: "/engine".into(),
        old: doc.engine.clone().unwrap_or(serde_json::Value::Null),
        new: serde_json::to_value(next).map_err(|_| TransportError::Internal)?,
        remove: false,
    };
    room.commit_ops_locked(
        repo,
        ctx,
        vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![change],
        }],
        now,
        WriteOrigin::AudioTransport,
    )
    .await
    .map_err(|_| TransportError::Internal)?;
    Ok(())
}

/// Server rule: when the world's active scene changes (`world-settings.activeScene` commits,
/// detected by `Room::publish`'s own hook — see that function's own doc), stop the PREVIOUS
/// scene's ambience entries (matched by playlist id) and start the NEW scene's ambience
/// playlist, if it declares one and it is not already playing. Best-effort: any read/parse
/// failure along the way silently skips that half (never blocks the activeScene write itself,
/// which has already committed by the time this runs).
pub(crate) async fn on_active_scene(
    repo: &dyn Repository,
    ctx: &PermissionContext,
    room: &Room,
    world_id: Uuid,
    old_active: Option<Uuid>,
    new_active: Option<Uuid>,
    now: i64,
) {
    let Ok(docs) = repo.query_documents(world_id, AUDIO_STATE_DOC_TYPE).await else {
        return;
    };
    let Some(doc) = docs.into_iter().next() else {
        return;
    };
    let state: AudioStateEngine = eng::engine_of(&doc);
    let mut next = state.clone();

    if let Some(old_id) = old_active {
        if let Some(ambience) = scene_ambience(repo, old_id).await {
            next.playing
                .retain(|t| t.playlist != Some(ambience.playlist));
        }
    }
    if let Some(new_id) = new_active {
        if let Some(ambience) = scene_ambience(repo, new_id).await {
            let already = next
                .playing
                .iter()
                .any(|t| t.playlist == Some(ambience.playlist));
            if !already {
                if let Ok(Some(pl_doc)) = repo.get_document(ambience.playlist).await {
                    if let Some(v) = &pl_doc.engine {
                        if let Ok(pl) = serde_json::from_value::<PlaylistEngine>(v.clone()) {
                            let op = AudioOp::Play {
                                playlist: Some(ambience.playlist),
                                asset: None,
                                track_index: None,
                                channel: Some(AudioChannel::Ambience),
                                gain: Some(ambience.gain),
                                loop_: None,
                            };
                            let lookup = |id: Uuid| (id == ambience.playlist).then(|| pl.clone());
                            if let Ok(applied) =
                                crate::audio::state::apply(&next, &op, now as f64, &lookup)
                            {
                                next = applied;
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = commit_audio_state(repo, ctx, room, &doc, &state, &next, now).await;
}

/// `scene`'s `SceneEngine.ambience`, or `None` on any read/parse failure or absence.
async fn scene_ambience(repo: &dyn Repository, scene: Uuid) -> Option<eng::scene::SceneAmbience> {
    let doc = repo.get_document(scene).await.ok().flatten()?;
    let v = doc.engine?;
    serde_json::from_value::<eng::SceneEngine>(v).ok()?.ambience
}
