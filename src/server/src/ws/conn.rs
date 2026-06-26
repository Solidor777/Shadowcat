//! WebSocket upgrade and per-connection ingress/egress tasks.
//!
//! All socket writes happen in the egress task (it owns the sink). The ingress
//! task parses client frames and forwards intents to egress over an mpsc
//! channel, or publishes directly to the room. The egress task multiplexes the
//! lossy broadcast stream (with a sequence guard + lag-driven resync) and the
//! ingress intent channel onto the one socket.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::stream::StreamExt;
use futures_util::{Sink, SinkExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::auth::session::AuthUser;
use crate::data::document::WorldCapDefaults;
use crate::data::membership::PermissionContext;
use crate::data::permission::filter_command;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::http::AppState;
use crate::ws::protocol::{ClientMsg, RejectReason, ServerMsg, WsErrorCode};
use crate::ws::room::Room;
use crate::ws::time::now_millis;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub world: Uuid,
}

/// Intents the ingress task hands to the egress task (which owns the sink).
enum Egress {
    Frame(Arc<ServerMsg>),
    TimePong {
        client_t0: i64,
        server_t: i64,
    },
    Resync(i64),
    /// Register a live search subscription (the egress task owns the registry).
    Subscribe {
        request_id: Uuid,
        query: String,
        limit: u32,
    },
    /// Cancel a live search subscription.
    Unsubscribe {
        request_id: Uuid,
    },
    /// Register a derived scene-channel subscription (egress-owned). `as_user` (GM-only
    /// see-as-player) is authorized + resolved in the egress handler.
    SceneSubscribe {
        request_id: Uuid,
        channel: String,
        as_user: Option<Uuid>,
    },
    /// Cancel a derived scene-channel subscription.
    SceneUnsubscribe {
        request_id: Uuid,
    },
}

/// Max live search subscriptions per connection; a subscribe beyond this is
/// rejected with `SearchError`.
const MAX_SUBSCRIPTIONS: usize = 16;
/// Max derived scene-channel subscriptions per connection.
const MAX_SCENE_SUBSCRIPTIONS: usize = 16;
/// Coalescing window: a burst of Events triggers at most one re-run per window.
const SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// A live search subscription's stored state.
struct Sub {
    query: String,
    limit: u32,
    /// Last delivered result identity, in rank order. Used to suppress a push
    /// when re-evaluation yields an identical top-N.
    fingerprint: Vec<(Uuid, u64, i64)>,
}

/// A derived scene-channel subscription's stored state. `fingerprint` is the
/// last delivered payload; a re-eval pushes only when it changes. `view_ctx` is the effective
/// context the channel is computed for: the connection's own ctx, or — for a GM see-as-player
/// subscription (M9c-2) — the server-resolved target player's context.
struct SceneSub {
    channel: String,
    fingerprint: Option<serde_json::Value>,
    view_ctx: PermissionContext,
}

/// A cheap, order-sensitive identity of a result page for no-op suppression:
/// `(doc_id, score-bits, updated_at)` per hit. Including `updated_at` makes a
/// content edit that leaves rank/score unchanged still push a fresh snippet.
fn search_fingerprint(hits: &[crate::data::search::SearchHit]) -> Vec<(Uuid, u64, i64)> {
    hits.iter()
        .map(|h| (h.document.id, h.score.to_bits(), h.document.updated_at))
        .collect()
}

/// Session-gated upgrade. `AuthUser` enforces authentication (401 without a
/// session) before the socket is established.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    user: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, user.id, user.role, q.world))
}

/// Serialize a server frame to a text WS message. Serializing our own types
/// never fails.
fn text(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

/// Map a write-path error to the client-actionable reject category.
fn reject_reason(e: &crate::data::DataError) -> RejectReason {
    use crate::data::DataError::*;
    match e {
        Forbidden => RejectReason::Forbidden,
        Conflict(_) => RejectReason::Conflict,
        _ => RejectReason::Invalid,
    }
}

/// Filter an outgoing frame for `ctx` and send it. Only `Event` frames carry
/// document data, so only they are redacted (per-recipient, seq-preserving);
/// every other frame passes through unchanged.
async fn send_filtered<S>(
    sink: &mut S,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    msg: &ServerMsg,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    let out = match msg {
        ServerMsg::Event { command, intent_id } => ServerMsg::Event {
            command: filter_command(repo, command, ctx, world_defaults).await,
            intent_id: *intent_id,
        },
        other => other.clone(),
    };
    sink.send(text(&out)).await.map_err(|_| ())
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user_id: Uuid,
    user_role: ServerRole,
    world_id: Uuid,
) {
    let repo = state.repo.clone();
    let room = match state.ws.rooms.get_or_create(repo.as_ref(), world_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            let mut s = socket;
            let _ = s
                .send(text(&ServerMsg::Error {
                    code: WsErrorCode::WorldNotFound,
                    message: "world not found".into(),
                }))
                .await;
            let _ = s.send(Message::Close(None)).await;
            return;
        }
        Err(_) => {
            let mut s = socket;
            let _ = s
                .send(text(&ServerMsg::Error {
                    code: WsErrorCode::Internal,
                    message: "internal".into(),
                }))
                .await;
            return;
        }
    };

    // Membership gate: a non-member non-admin cannot build a PermissionContext,
    // so cannot join. The context, resolved once here, authorizes writes and
    // filters every outgoing frame for the rest of the connection.
    let ctx = match repo.permission_context(world_id, user_id, user_role).await {
        Ok(c) => c,
        Err(_) => {
            let mut s = socket;
            let _ = s
                .send(text(&ServerMsg::Error {
                    code: WsErrorCode::Forbidden,
                    message: "not a member of this world".into(),
                }))
                .await;
            let _ = s.send(Message::Close(None)).await;
            tracing::info!(world = %world_id, user = %user_id, "ws join denied: not a member");
            return;
        }
    };

    room.stats.connections.fetch_add(1, Ordering::AcqRel);
    tracing::info!(world = %world_id, user = %user_id, "ws connected");
    let (rx, current_seq) = room.subscribe();
    let (sink, mut stream) = socket.split();
    let (etx, erx) = mpsc::channel::<Egress>(64);

    // Egress task owns the sink: sends Welcome, then multiplexes broadcast +
    // ingress intents with a per-connection sequence guard.
    let egress_room = room.clone();
    let egress_repo = repo.clone();
    let mut egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        egress_room,
        egress_repo,
        ctx,
        current_seq,
    ));

    // Ingress: parse client frames, forward intents to egress / publish.
    // Per-user ping budget (shared across this user's connections; survives reconnect).
    let ping_rate = state.ws.ping_rate.clone();
    loop {
        tokio::select! {
            _ = &mut egress => break,
            frame = stream.next() => {
                let Some(Ok(frame)) = frame else { break };
                match frame {
                    Message::Text(t) => match serde_json::from_str::<ClientMsg>(t.as_str()) {
                        Ok(ClientMsg::Intent { intent_id, ops }) => {
                            // Success is confirmed by the broadcast echo of the
                            // authored Event; only a rejection is sent directly.
                            match room.publish(repo.as_ref(), &ctx, ops, now_millis()).await {
                                Ok(_cmd) => {}
                                Err(e) => {
                                    let reason = reject_reason(&e);
                                    tracing::debug!(world = %world_id, %intent_id, ?reason, "intent rejected");
                                    let _ = etx
                                        .send(Egress::Frame(Arc::new(ServerMsg::Reject {
                                            intent_id,
                                            reason,
                                        })))
                                        .await;
                                }
                            }
                        }
                        Ok(ClientMsg::TimePing { client_t0 }) => {
                            if etx
                                .send(Egress::TimePong { client_t0, server_t: now_millis() })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(ClientMsg::ResyncRequest { from_seq }) => {
                            if etx.send(Egress::Resync(from_seq)).await.is_err() {
                                break;
                            }
                        }
                        Ok(ClientMsg::Search { request_id, query, limit, cursor, subscribe }) => {
                            if subscribe {
                                // Subscriptions are owned by the egress task (it has
                                // the registry, the broadcast, and the sink).
                                if etx.send(Egress::Subscribe { request_id, query, limit }).await.is_err() {
                                    break;
                                }
                            } else {
                                let from = cursor.as_deref().and_then(|c| c.parse::<i64>().ok());
                                let frame = match repo.search(&ctx, world_id, &query, limit, from).await {
                                    Ok(page) => ServerMsg::SearchResult {
                                        request_id,
                                        hits: page.hits,
                                        next_cursor: page.next_cursor.map(|n| n.to_string()),
                                    },
                                    Err(e) => {
                                        tracing::debug!(world = %world_id, %request_id, error = %e, "search failed");
                                        ServerMsg::SearchError {
                                            request_id,
                                            message: "search failed".into(),
                                        }
                                    }
                                };
                                if etx.send(Egress::Frame(Arc::new(frame))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::Unsubscribe { request_id }) => {
                            if etx.send(Egress::Unsubscribe { request_id }).await.is_err() {
                                break;
                            }
                        }
                        Ok(ClientMsg::Hello { .. }) | Ok(ClientMsg::Pong) => {}
                        Ok(ClientMsg::SceneSubscribe { request_id, channel, as_user }) => {
                            if etx
                                .send(Egress::SceneSubscribe { request_id, channel, as_user })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(ClientMsg::SceneUnsubscribe { request_id }) => {
                            if etx
                                .send(Egress::SceneUnsubscribe { request_id })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(ClientMsg::ScenePing { scene, x, y }) => {
                            // Out-of-band relay to the world room, stamped with the sender.
                            // Membership is already gated (a non-member never reaches here);
                            // coordinates are not validated. Over-budget pings drop silently.
                            if ping_rate.check(user_id, now_millis(), 30) {
                                room.broadcast_aux(ServerMsg::ScenePing {
                                    scene,
                                    x,
                                    y,
                                    user: user_id,
                                });
                            }
                        }
                        Ok(ClientMsg::MoveRequest { request_id, scene, token_id, path }) => {
                            // Server-authoritative move execution. On success, broadcasts
                            // MoveStream out-of-band to the room — no etx reply to the requester.
                            // On failure, returns MoveError to etx only (no geometry leak).
                            // INVARIANT (broadcast-not-requester): the atomic position Event
                            // from commit_ops_locked + the MoveStream broadcast are the
                            // notifications; no success frame is sent to the requester's etx.
                            if let Some(err_frame) = handle_move_request(
                                &room,
                                repo.as_ref(),
                                &ctx,
                                scene,
                                token_id,
                                path,
                                request_id,
                            )
                            .await
                            {
                                if etx.send(Egress::Frame(Arc::new(err_frame))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::Pathfind { request_id, scene, start, waypoints, footprint_radius }) => {
                            // One-shot pathfinding: resolve GM status, fetch explored off the lock for
                            // non-GM Revealed, call SceneEcs::pathfind, reply to this connection only.
                            // INVARIANT (one-shot-to-requester): reply goes to etx only, never broadcast.
                            let frame = handle_pathfind(
                                request_id, scene, start, waypoints, footprint_radius,
                                &ctx, &room, repo.as_ref(),
                            ).await;
                            if etx.send(Egress::Frame(Arc::new(frame))).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = etx
                                .send(Egress::Frame(Arc::new(ServerMsg::Error {
                                    code: WsErrorCode::BadMessage,
                                    message: "malformed frame".into(),
                                })))
                                .await;
                        }
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    egress.abort();
    room.stats.connections.fetch_sub(1, Ordering::AcqRel);
    state.ws.rooms.reap_if_empty(world_id);
    tracing::info!(world = %world_id, user = %user_id, "ws disconnected");
}

/// Resolve and execute a one-shot grid pathfind request.
///
/// INVARIANT (no-lock-across-await): the scene read guard is taken twice — once to read
/// `movement_restriction` (then dropped), and once to call `pathfind` (then dropped again) —
/// so `get_explored` can be awaited between them without holding the lock.
/// INVARIANT (one-shot-to-requester): the reply is placed directly on `etx`; it is never
/// broadcast to the room.
#[allow(clippy::too_many_arguments)]
async fn handle_pathfind(
    request_id: Uuid,
    scene: Uuid,
    start: (f64, f64),
    waypoints: Vec<(f64, f64)>,
    footprint_radius: f64,
    ctx: &crate::data::membership::PermissionContext,
    room: &crate::ws::room::Room,
    repo: &dyn crate::data::repository::Repository,
) -> ServerMsg {
    let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
    // Step 1: check movement_restriction under a short read guard, then drop it.
    let need_explored = !is_gm && {
        let s = room.scene().read().await;
        matches!(
            s.resolve_scene(scene).movement_restriction,
            crate::scene::MovementRestriction::Revealed
        )
    };
    // Step 2: fetch explored (if needed) after the lock is dropped.
    let explored = if need_explored {
        match repo.get_explored(scene, ctx.user_id).await {
            Ok(Some(blob)) => Some(crate::scene::explored::ExploredSet::from_bytes(&blob)),
            // Fail closed: Revealed degrades to visible-only on any error/miss.
            _ => None,
        }
    } else {
        None
    };
    // Step 3: take a fresh read guard to call pathfind.
    let s = room.scene().read().await;
    match s.pathfind(
        ctx.user_id,
        scene,
        start,
        &waypoints,
        footprint_radius,
        is_gm,
        explored.as_ref(),
    ) {
        Ok((path, cost)) => ServerMsg::PathResult {
            request_id,
            path,
            cost,
        },
        Err(e) => ServerMsg::PathError {
            request_id,
            message: match e {
                crate::scene::pathfinding::PathFail::Invalid => "invalid request",
                crate::scene::pathfinding::PathFail::Unreachable => "unreachable",
                crate::scene::pathfinding::PathFail::Exceeded => "search exceeded",
            }
            .to_string(),
        },
    }
}

/// Resolve and execute a server-authoritative one-shot move request.
///
/// INVARIANT (broadcast-not-requester): on success, broadcasts `MoveStream` out-of-band to the
/// room via `broadcast_aux` (no seq, mirrors `ScenePing`). No success frame is returned to
/// the requester's `etx` — the broadcast IS the notification. The atomic position `Event`
/// from `commit_ops_locked` carries the authoritative position update for document-store sync.
/// INVARIANT (no-geometry-leak): on any `execute_move` failure the reply is a generic
/// `MoveError { message: "move rejected" }` to `etx` only — no path geometry or vision state
/// is disclosed.
/// INVARIANT (mover_vision): `exec.mover_vision` is `None` for GM movers (no fog to sweep)
/// and `Some` for player movers. It is mapped to wire `VisionSample` with per-polygon vertex
/// capping (fail-closed under-reveal: truncation never over-reveals hidden area).
async fn handle_move_request(
    room: &crate::ws::room::Room,
    repo: &dyn crate::data::repository::Repository,
    ctx: &crate::data::membership::PermissionContext,
    scene_id: Uuid,
    token_id: Uuid,
    // Ordered cell-center scene points: start … goal as `[f64; 2]` wire arrays.
    path: Vec<[f64; 2]>,
    request_id: Uuid,
) -> Option<ServerMsg> {
    // Convert wire `[f64; 2]` arrays to the internal `(f64, f64)` tuple representation
    // expected by `Room::execute_move`.
    let path_tuples: Vec<(f64, f64)> = path.iter().map(|p| (p[0], p[1])).collect();
    // Single clock capture: `now` is used both as the committed event timestamp and as
    // `start_server_ms` so the animation origin equals the commit instant — a second
    // `now_millis()` call after `execute_move` returns (after the DB write) would drift
    // `start_server_ms` forward from the actual commit timestamp.
    let now = now_millis();
    match room
        .execute_move(repo, ctx, scene_id, token_id, path_tuples, now)
        .await
    {
        Ok(exec) => {
            use crate::scene::move_stream::MAX_VISION_POLYGON_VERTS;
            use crate::ws::protocol::{PosSample, VisionSample};
            // Map internal VisionSamplePt → wire VisionSample, capping polygon vertex count.
            // Fail-closed: truncation under-reveals (the mover sees less of the fog sweep) but
            // never over-reveals hidden geometry to the client.
            let mover_vision = exec.mover_vision.map(|mvs| {
                mvs.into_iter()
                    .map(|vs| VisionSample {
                        t_ms: vs.t_ms,
                        polygons: vs
                            .polygons
                            .into_iter()
                            .map(|poly| {
                                poly.into_iter()
                                    .take(MAX_VISION_POLYGON_VERTS)
                                    .map(|(x, y)| [x, y])
                                    .collect()
                            })
                            .collect(),
                    })
                    .collect()
            });
            let frame = ServerMsg::MoveStream {
                request_id,
                token_id,
                mover: ctx.user_id,
                scene: scene_id,
                start_server_ms: now as f64,
                duration_ms: exec.duration_ms,
                stop: [exec.stop.0, exec.stop.1],
                samples: exec
                    .samples
                    .iter()
                    .map(|s| PosSample {
                        t_ms: s.t_ms,
                        pos: [s.pos.0, s.pos.1],
                    })
                    .collect(),
                mover_vision,
            };
            room.broadcast_aux(frame);
            // No success frame to the requester: the broadcast is the notification.
            None
        }
        Err(_) => Some(ServerMsg::MoveError {
            request_id,
            message: "move rejected".into(),
        }),
    }
}

/// Inject the player's scene-tagged `explored` cell sets into a `vision` **masked** payload, and —
/// when `accumulate` — mark the currently-visible cells into the player's stored explored and
/// persist on growth. No-op for a GM (`mode:"all"`) or any payload without masked polygons. Runs
/// after the ECS read lock is dropped (it does async DB I/O); `grid` carries each scene's cell size,
/// captured under that lock. Explored is emitted only for scenes the player currently has vision in
/// (the payload's polygons) — a token-less player gets no explored. `accumulate` is FALSE for a GM
/// see-as-player view (M9c-2): it is a read-only observer that emits the target's stored explored
/// but must NOT grow the target's memory from the GM's session.
async fn enrich_vision_explored(
    payload: &mut serde_json::Value,
    grid: &std::collections::HashMap<Uuid, f64>,
    repo: &SqliteRepository,
    world: Uuid,
    user: Uuid,
    accumulate: bool,
) {
    if payload.get("mode").and_then(|m| m.as_str()) != Some("masked") {
        return;
    }
    // Group the recipient's visibility polygons by scene (scene-local coords).
    let polys = payload
        .get("polygons")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let mut by_scene: std::collections::HashMap<Uuid, Vec<Vec<f64>>> =
        std::collections::HashMap::new();
    for poly in &polys {
        let Some(scene) = poly
            .get("scene")
            .and_then(|s| s.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let points: Vec<f64> = poly
            .get("points")
            .and_then(|p| p.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();
        by_scene.entry(scene).or_default().push(points);
    }
    let mut explored_out: Vec<serde_json::Value> = Vec::with_capacity(by_scene.len());
    for (scene, scene_polys) in by_scene {
        let cell = grid.get(&scene).copied().unwrap_or(100.0);
        let mut set = match repo.get_explored(scene, user).await {
            Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob),
            _ => crate::scene::explored::ExploredSet::new(),
        };
        if accumulate && set.mark_polygons(&scene_polys, cell) > 0 {
            let _ = repo.set_explored(world, scene, user, &set.to_bytes()).await;
        }
        let cells: Vec<i32> = set.iter().flat_map(|(i, j)| [i, j]).collect();
        explored_out.push(serde_json::json!({ "scene": scene, "cell": cell, "cells": cells }));
    }
    payload["explored"] = serde_json::json!(explored_out);
}

/// Return the recipient's authoritative vision polygons that cover `scene`.
///
/// Always reads from the authoritative ECS `player_vision_polygons` — a rendering cache
/// is NOT a secrecy gate: if the observer's vision shrank within the ~150 ms debounce
/// window a stale, wider polygon would admit a now-hidden sample. One ECS read per
/// MoveStream per observer is acceptable for a security gate.
///
/// Returns empty on any failure or when the recipient controls no token in `scene`
/// (fail-closed: caller suppresses the frame).
///
/// INVARIANT (no-lock-across-await): the ECS read guard is held only for the synchronous
/// `player_vision_polygons` call and is dropped before this `async fn` returns, so no
/// lock survives to the `sink.send` await in the egress loop.
async fn observer_vision_polys_for_scene(
    user_id: Uuid,
    scene: Uuid,
    room: &crate::ws::room::Room,
) -> Vec<Vec<crate::scene::vision::P>> {
    // Authoritative ECS read. Drop the lock before returning so no lock crosses
    // the downstream `sink.send` await.
    let polys_all = {
        let ecs = room.scene().read().await;
        ecs.player_vision_polygons(user_id)
    };
    polys_all
        .into_iter()
        .filter(|(s, _)| *s == scene)
        .map(|(_, poly)| poly)
        .collect()
}

/// Per-recipient `MoveStream` clip — the egress secrecy boundary.
///
/// Returns `Some(clipped)` when the recipient may see ≥1 position sample, `None` to
/// suppress the frame entirely.
///
/// Discrimination:
/// - **Mover** (`ctx.user_id == frame.mover`): full frame forwarded unchanged (all
///   samples + `mover_vision`).
/// - **GM** (world role): all samples forwarded, `mover_vision` nulled, full `stop`
///   and `duration_ms` preserved.
/// - **Observer**: only samples whose `pos` lies within the recipient's authoritative
///   vision polygons are forwarded; `mover_vision` nulled; fully-occluded → `None`.
///   `stop` and `duration_ms` are clipped to the LAST VISIBLE sample — the true final
///   position and full travel distance are not disclosed.
///
/// INVARIANT (mover_vision-isolation): `mover_vision` reaches only the mover's socket.
/// INVARIANT (fail-closed): no derivable vision → empty clip → suppress.
/// INVARIANT (no-stale-cache): observer vision is always read from the authoritative ECS,
///   never from a rendering-cache fingerprint (a stale wider polygon would admit a
///   now-hidden sample).
/// INVARIANT (no-lock-across-await): the ECS read lock (if taken) is dropped inside
///   `observer_vision_polys_for_scene` before this function returns.
async fn clip_move_stream(
    msg: &ServerMsg,
    ctx: &PermissionContext,
    room: &crate::ws::room::Room,
) -> Option<ServerMsg> {
    let ServerMsg::MoveStream {
        request_id,
        token_id,
        mover,
        scene,
        start_server_ms,
        duration_ms,
        stop,
        samples,
        mover_vision: _, // forwarded only to the mover via msg.clone(); observers get None
    } = msg
    else {
        return None;
    };

    // Mover receives their own stream unchanged (all samples + mover_vision).
    if ctx.user_id == *mover {
        return Some(msg.clone());
    }

    // GM: all position samples pass, but mover sightlines are never disclosed.
    if ctx.world_role == crate::data::document::WorldRole::Gm {
        return Some(ServerMsg::MoveStream {
            request_id: *request_id,
            token_id: *token_id,
            mover: *mover,
            scene: *scene,
            start_server_ms: *start_server_ms,
            duration_ms: *duration_ms,
            stop: *stop,
            samples: samples.clone(),
            mover_vision: None,
        });
    }

    // Observer: clip to samples the recipient can see within their authoritative vision.
    // The ECS read is dropped inside `observer_vision_polys_for_scene` before this
    // function returns, so no lock crosses the `sink.send` await in the caller.
    let polys = observer_vision_polys_for_scene(ctx.user_id, *scene, room).await;
    use crate::scene::vision::point_in_poly;
    use crate::ws::protocol::PosSample;
    let visible: Vec<PosSample> = samples
        .iter()
        .filter(|s| {
            let p = (s.pos[0], s.pos[1]);
            polys.iter().any(|poly| point_in_poly(poly, p))
        })
        .copied()
        .collect();
    if visible.is_empty() {
        return None; // SUPPRESS: fully occluded or no vision available (fail-closed)
    }
    // Clip stop and duration_ms to the last VISIBLE sample so the observer learns
    // neither the true final position (which may be behind a wall) nor the full
    // travel distance. The authoritative position Event (from commit_ops_locked)
    // delivers the real stop coordinate later, gated by the client's fog layer.
    let clipped_stop = visible.last().map(|s| s.pos).unwrap_or(*stop);
    let clipped_duration_ms = visible.last().map(|s| s.t_ms).unwrap_or(*duration_ms);
    Some(ServerMsg::MoveStream {
        request_id: *request_id,
        token_id: *token_id,
        mover: *mover,
        scene: *scene,
        start_server_ms: *start_server_ms,
        duration_ms: clipped_duration_ms,
        stop: clipped_stop,
        samples: visible,
        mover_vision: None, // INVARIANT: mover_vision strictly mover-only
    })
}

async fn egress_loop<S>(
    mut sink: S,
    mut rx: tokio::sync::broadcast::Receiver<Arc<ServerMsg>>,
    mut erx: mpsc::Receiver<Egress>,
    room: Arc<Room>,
    repo: Arc<SqliteRepository>,
    ctx: PermissionContext,
    current_seq: i64,
) where
    S: Sink<Message> + Unpin,
{
    let world_id = room.world_id;
    // Loaded once per connection (not per event): a per-event read would contend
    // with apply_intent on the single-writer pool. A defaults change mid-session
    // takes effect on the client's next (re)connect.
    let world_defaults = repo.world_cap_defaults(world_id).await.unwrap_or_default();
    let world_reqs = match repo.world_cap_requirements(world_id).await {
        Ok(r) => r,
        Err(e) => {
            // Fail open for the advisory client copy only; server-side
            // enforcement reads requirements freshly per intent and fails closed.
            tracing::warn!(world = %world_id, error = %e, "capability requirements unreadable; sending empty");
            Vec::new()
        }
    };
    let world_contracts = match repo.world_contract_declarations(world_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "contract declarations unreadable; sending empty");
            Vec::new()
        }
    };
    // Project the world grants to only what this actor needs to self-gate; other
    // users' UUIDs and grants must not cross to the client.
    let actor_grants =
        crate::data::permission::project_grants_for(&world_defaults.all, ctx.user_id);
    if sink
        .send(text(&ServerMsg::Welcome {
            world: world_id,
            current_seq,
            server_time: now_millis(),
            world_default_grants: actor_grants,
            user_role: ctx.world_role,
            capability_requirements: world_reqs,
            contract_declarations: world_contracts,
        }))
        .await
        .is_err()
    {
        return;
    }

    // Live search subscriptions owned by this connection. Each authoritative
    // Event arms a debounce; on fire, every subscription is re-run against the
    // current state with THIS connection's ctx (so per-recipient filtering and
    // the visibility-split index apply) and pushed only if its result changed.
    let mut subs: std::collections::HashMap<Uuid, Sub> = std::collections::HashMap::new();
    let mut scene_subs: std::collections::HashMap<Uuid, SceneSub> =
        std::collections::HashMap::new();
    let mut reeval_deadline: Option<tokio::time::Instant> = None;

    let mut next_expected = current_seq + 1;
    loop {
        tokio::select! {
            cmd = erx.recv() => match cmd {
                Some(Egress::Frame(f)) => {
                    if send_filtered(&mut sink, repo.as_ref(), &ctx, &world_defaults, f.as_ref()).await.is_err() { break; }
                }
                Some(Egress::TimePong { client_t0, server_t }) => {
                    if sink.send(text(&ServerMsg::TimePong { client_t0, server_t })).await.is_err() { break; }
                }
                Some(Egress::Resync(from)) => {
                    match replay(&mut sink, &room, repo.as_ref(), &ctx, &world_defaults, from).await {
                        Ok(to_seq) => next_expected = (to_seq + 1).max(next_expected),
                        Err(_) => break,
                    }
                }
                Some(Egress::Subscribe { request_id, query, limit }) => {
                    if subs.contains_key(&request_id) {
                        // A duplicate id would silently orphan the prior sub.
                        let f = ServerMsg::SearchError { request_id, message: "duplicate subscription id".into() };
                        if sink.send(text(&f)).await.is_err() { break; }
                    } else if subs.len() >= MAX_SUBSCRIPTIONS {
                        let f = ServerMsg::SearchError { request_id, message: "too many subscriptions".into() };
                        if sink.send(text(&f)).await.is_err() { break; }
                    } else {
                        match repo.search(&ctx, world_id, &query, limit, None).await {
                            Ok(page) => {
                                let fp = search_fingerprint(&page.hits);
                                let f = ServerMsg::SearchResult { request_id, hits: page.hits, next_cursor: None };
                                if sink.send(text(&f)).await.is_err() { break; }
                                subs.insert(request_id, Sub { query, limit, fingerprint: fp });
                            }
                            Err(e) => {
                                tracing::debug!(world = %world_id, %request_id, error = %e, "subscribe search failed");
                                let f = ServerMsg::SearchError { request_id, message: "search failed".into() };
                                if sink.send(text(&f)).await.is_err() { break; }
                            }
                        }
                    }
                }
                Some(Egress::Unsubscribe { request_id }) => {
                    subs.remove(&request_id);
                }
                Some(Egress::SceneSubscribe { request_id, channel, as_user }) => {
                    if scene_subs.contains_key(&request_id) {
                        // A duplicate id would silently orphan the prior sub (mirrors the search path).
                        let f = ServerMsg::SceneError { request_id, message: "duplicate subscription id".into() };
                        if sink.send(text(&f)).await.is_err() { break; }
                    } else if scene_subs.len() >= MAX_SCENE_SUBSCRIPTIONS {
                        let f = ServerMsg::SceneError { request_id, message: "too many subscriptions".into() };
                        if sink.send(text(&f)).await.is_err() { break; }
                    } else {
                        // Resolve the effective view context. `as_user` (see-as-player, M9c-2) is
                        // GM-ONLY, and the target's role is resolved SERVER-SIDE — a non-GM can never
                        // view as another user, and a client-supplied role/scope is never trusted.
                        // This is the player-to-player access boundary.
                        let view_ctx = match as_user {
                            None => ctx,
                            Some(target) => {
                                if ctx.world_role != crate::data::document::WorldRole::Gm {
                                    let f = ServerMsg::SceneError { request_id, message: "not authorized to view as another user".into() };
                                    if sink.send(text(&f)).await.is_err() { break; }
                                    continue;
                                }
                                match repo.member_role(world_id, target).await {
                                    Ok(Some(role)) => PermissionContext { user_id: target, world_role: role },
                                    _ => {
                                        let f = ServerMsg::SceneError { request_id, message: "target user is not a member of this world".into() };
                                        if sink.send(text(&f)).await.is_err() { break; }
                                        continue;
                                    }
                                }
                            }
                        };
                        // Persist explored only for the connection's OWN view; a GM see-as is a
                        // read-only observer that must not grow the target player's memory.
                        let accumulate = view_ctx.user_id == ctx.user_id;
                        // Read the ECS and the seq it reflects under one borrow, then drop it before
                        // awaiting the sink. Grid sizes are captured under the same lock for the
                        // post-lock explored step. Computed for `view_ctx` (own, or the see-as target).
                        let (payload, seq, grid) = {
                            let ecs = room.scene().read().await;
                            (crate::scene::compute_derived(&channel, &ecs, &view_ctx), ecs.committed_seq(), ecs.scene_grid_sizes())
                        };
                        match payload {
                            Some(mut p) => {
                                if channel == "vision" {
                                    enrich_vision_explored(&mut p, &grid, repo.as_ref(), world_id, view_ctx.user_id, accumulate).await;
                                }
                                let f = ServerMsg::SceneDerived {
                                    request_id,
                                    channel: channel.clone(),
                                    computed_at_seq: seq,
                                    payload: p.clone(),
                                };
                                if sink.send(text(&f)).await.is_err() { break; }
                                scene_subs.insert(request_id, SceneSub { channel, fingerprint: Some(p), view_ctx });
                            }
                            None => {
                                let f = ServerMsg::SceneError { request_id, message: format!("unknown channel: {channel}") };
                                if sink.send(text(&f)).await.is_err() { break; }
                            }
                        }
                    }
                }
                Some(Egress::SceneUnsubscribe { request_id }) => {
                    scene_subs.remove(&request_id);
                }
                None => break, // ingress gone
            },
            msg = rx.recv() => match msg {
                Ok(msg) => {
                    if let Some(seq) = msg.event_seq() {
                        if seq < next_expected {
                            continue; // already delivered via a resync
                        }
                        if seq > next_expected {
                            room.stats.gaps_detected.fetch_add(1, Ordering::Relaxed);
                            tracing::debug!(world = %world_id, expected = next_expected, got = seq, "gap detected");
                            match replay(&mut sink, &room, repo.as_ref(), &ctx, &world_defaults, next_expected).await {
                                Ok(to_seq) => next_expected = to_seq + 1,
                                Err(_) => break,
                            }
                            if seq < next_expected { continue; }
                        }
                        if send_filtered(&mut sink, repo.as_ref(), &ctx, &world_defaults, msg.as_ref()).await.is_err() { break; }
                        next_expected = seq + 1;
                        // A world change may affect live subscriptions. Arm the
                        // coalescing window on the LEADING edge only: re-arming
                        // on every Event would push the deadline forward forever
                        // under a sustained stream (starving updates). Arming
                        // only when idle fires ~150ms after the first Event of a
                        // burst, then re-arms on the next Event after it fires.
                        if (!subs.is_empty() || !scene_subs.is_empty())
                            && reeval_deadline.is_none()
                        {
                            reeval_deadline = Some(tokio::time::Instant::now() + SEARCH_DEBOUNCE);
                        }
                    } else {
                        // Non-Event, non-sequenced out-of-band frame. `MoveStream` requires
                        // per-recipient egress clipping (the secrecy boundary); every other
                        // frame passes through the generic permission filter unchanged.
                        let should_break = match msg.as_ref() {
                            ServerMsg::MoveStream { .. } => {
                                match clip_move_stream(msg.as_ref(), &ctx, &room).await {
                                    Some(out) => sink.send(text(&out)).await.is_err(),
                                    None => false, // suppressed: do not send
                                }
                            }
                            other => send_filtered(
                                &mut sink,
                                repo.as_ref(),
                                &ctx,
                                &world_defaults,
                                other,
                            )
                            .await
                            .is_err(),
                        };
                        if should_break {
                            break;
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    room.stats.lagged_drops.fetch_add(n, Ordering::Relaxed);
                    tracing::warn!(world = %world_id, dropped = n, "broadcast lagged");
                    match replay(&mut sink, &room, repo.as_ref(), &ctx, &world_defaults, next_expected).await {
                        Ok(to_seq) => next_expected = to_seq + 1,
                        Err(_) => break,
                    }
                }
                Err(RecvError::Closed) => break,
            },
            // Coalesced live-search re-evaluation: fires ~one debounce window
            // after the first Event of a burst. Re-runs each subscription with
            // this actor's ctx and pushes only when the result changed (no-op
            // suppression). Cost is bounded — at most MAX_SUBSCRIPTIONS searches,
            // each capped by the search scan budget, at most once per window —
            // but it runs inline on the egress task. TODO: offload re-eval reads
            // off the egress path (a read pool / spawned task) if busy worlds
            // show broadcast lag from this coupling.
            _ = tokio::time::sleep_until(reeval_deadline.unwrap_or_else(tokio::time::Instant::now)),
                if reeval_deadline.is_some() =>
            {
                reeval_deadline = None;
                let mut dead: Vec<Uuid> = Vec::new();
                for (id, sub) in subs.iter_mut() {
                    match repo.search(&ctx, world_id, &sub.query, sub.limit, None).await {
                        Ok(page) => {
                            let fp = search_fingerprint(&page.hits);
                            if fp != sub.fingerprint {
                                sub.fingerprint = fp;
                                let f = ServerMsg::SearchUpdate { request_id: *id, hits: page.hits };
                                // `return` (not `break`): a bare break would only
                                // exit this inner for-loop, leaving the egress
                                // loop running on a dead sink. Other arms `break`
                                // the egress loop directly; here the send is
                                // nested, so end the task outright.
                                if sink.send(text(&f)).await.is_err() { return; }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(world = %world_id, subscription = %id, error = %e, "live re-eval failed");
                            let f = ServerMsg::SearchError { request_id: *id, message: "search failed".into() };
                            let _ = sink.send(text(&f)).await;
                            dead.push(*id);
                        }
                    }
                }
                for id in dead {
                    subs.remove(&id);
                }
                // Re-evaluate derived scene subscriptions against the current ECS, each with its
                // own effective view ctx (own, or a GM see-as target); push only when a channel's
                // payload changed. The read borrow is dropped before awaiting the sink.
                let (seq, snapshot, grid) = {
                    let ecs = room.scene().read().await;
                    let mut out = Vec::new();
                    for (id, s) in scene_subs.iter() {
                        out.push((
                            *id,
                            s.channel.clone(),
                            s.view_ctx,
                            crate::scene::compute_derived(&s.channel, &ecs, &s.view_ctx),
                        ));
                    }
                    (ecs.committed_seq(), out, ecs.scene_grid_sizes())
                };
                for (id, channel, view_ctx, payload) in snapshot {
                    if let Some(mut p) = payload {
                        if channel == "vision" {
                            // See-as (view_ctx != own) is read-only: emit the target's explored, never persist.
                            let accumulate = view_ctx.user_id == ctx.user_id;
                            enrich_vision_explored(&mut p, &grid, repo.as_ref(), world_id, view_ctx.user_id, accumulate).await;
                        }
                        if let Some(sub) = scene_subs.get_mut(&id) {
                            if sub.fingerprint.as_ref() != Some(&p) {
                                sub.fingerprint = Some(p.clone());
                                let f = ServerMsg::SceneDerived {
                                    request_id: id,
                                    channel,
                                    computed_at_seq: seq,
                                    payload: p,
                                };
                                if sink.send(text(&f)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Replay `[from_seq, to_seq]` to the sink as ResyncBegin .. Event* .. ResyncEnd,
/// where `to_seq` is the last seq actually sent (a point-in-time snapshot taken by
/// `resync_range`). Returns `to_seq` so the caller advances its watermark to
/// exactly what was delivered — NOT a fresh `current_seq` read, which can race
/// ahead of the snapshot and silently drop events published during this replay's
/// I/O. `ResyncEnd.current_seq` reports the same `to_seq` so the client's
/// watermark matches; events after `to_seq` arrive via normal live delivery.
async fn replay<S>(
    sink: &mut S,
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    from_seq: i64,
) -> Result<i64, ()>
where
    S: Sink<Message> + Unpin,
{
    let (frames, source) = room.resync_range(repo, from_seq).await.map_err(|_| ())?;
    let to_seq = frames
        .last()
        .and_then(|m| m.event_seq())
        .unwrap_or(from_seq - 1);
    tracing::debug!(from_seq, to_seq, ?source, "resync served");
    sink.send(text(&ServerMsg::ResyncBegin {
        from_seq,
        to_seq,
        source,
    }))
    .await
    .map_err(|_| ())?;
    // Replayed events are redacted per recipient, identically to live delivery.
    for f in frames {
        send_filtered(sink, repo, ctx, world_defaults, f.as_ref()).await?;
    }
    sink.send(text(&ServerMsg::ResyncEnd {
        current_seq: to_seq,
    }))
    .await
    .map_err(|_| ())?;
    Ok(to_seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Deterministic broadcast-`Lagged` → resync guard, driven directly against the
    /// generic `egress_loop` with a credit-gated in-process sink — no real socket, so
    /// it does not depend on any OS's TCP buffer sizing (the prior socket-backpressure
    /// approach was non-portable: `SO_SNDBUF`/`SO_RCVBUF` are advisory and each OS
    /// clamps/autotunes them differently). The sink starts with exactly one credit
    /// (consumed by `Welcome`); with zero credits the egress drains at most one
    /// broadcast event before parking on the gated send, so publishing
    /// `30 >> capacity(8)` events overflows the broadcast ring deterministically.
    /// Granting credits unblocks the egress, which then observes `Lagged`, replays
    /// from the ring/log, and converges to the authoritative tail with no gaps/dups.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn egress_lag_triggers_resync_and_converges() {
        use crate::data::command::Operation;
        use crate::data::document::WorldRole;
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use tokio::sync::Semaphore;

        // A `Sink<Message>` whose readiness is gated by a semaphore credit; accepted
        // frames are forwarded to an unbounded channel the test drains. Each send
        // consumes one credit (the permit is `forget`-ten), so the test controls
        // exactly how many frames the egress may emit, and thus when it stalls.
        struct GatedSink {
            out: mpsc::UnboundedSender<Message>,
            credits: Arc<Semaphore>,
            acquiring: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
        }
        impl Sink<Message> for GatedSink {
            type Error = ();
            fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
                let this = self.as_mut().get_mut();
                if this.acquiring.is_none() {
                    let sem = this.credits.clone();
                    // The semaphore never closes in-test, so the acquire cannot fail.
                    this.acquiring = Some(Box::pin(async move {
                        sem.acquire_owned().await.unwrap().forget()
                    }));
                }
                match this.acquiring.as_mut().unwrap().as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        this.acquiring = None;
                        Poll::Ready(Ok(()))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
            fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), ()> {
                let _ = self.get_mut().out.send(item);
                Ok(())
            }
            fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
                Poll::Ready(Ok(()))
            }
        }

        fn msg_text(m: &Message) -> &str {
            match m {
                Message::Text(t) => t.as_str(),
                _ => "",
            }
        }

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let author = repo
            .create_user("a", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", author, 0).await.unwrap();
        let ctx = PermissionContext {
            user_id: author,
            world_role: WorldRole::Gm,
        };

        // Ring capacity 8: fewer than the 30 events published while the egress is gated.
        let reg = crate::ws::room::RoomRegistry::with_capacity(8);
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let (rx, current_seq) = room.subscribe();

        let credits = Arc::new(Semaphore::new(1)); // one credit: the `Welcome` send
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
        // Held open so the egress never sees its ingress channel close mid-test.
        let (etx, erx) = mpsc::channel::<Egress>(8);
        let sink = GatedSink {
            out: out_tx,
            credits: credits.clone(),
            acquiring: None,
        };
        let egress = tokio::spawn(egress_loop(
            sink,
            rx,
            erx,
            room.clone(),
            repo.clone(),
            ctx,
            current_seq,
        ));

        // Drain the `Welcome` (consumes the sole credit); the egress now has zero
        // credits, so it can pull at most one broadcast event before parking.
        let welcome = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
            .await
            .expect("egress did not emit Welcome")
            .expect("egress sink closed before Welcome");
        let wv: serde_json::Value = serde_json::from_str(msg_text(&welcome)).unwrap();
        assert_eq!(wv["type"], "welcome");

        // Publish 30 world docs. With the egress gated, far more than capacity(8)
        // accumulate unread in the broadcast ring and overflow it.
        for n in 0..30u128 {
            let mut doc = crate::data::document::tests::world_scoped_doc(
                world.id,
                Uuid::from_u128(1000 + n),
                "actor",
            );
            doc.owner = Some(author);
            room.publish(repo.as_ref(), &ctx, vec![Operation::Create { doc }], 0)
                .await
                .unwrap();
        }

        // Release the gate; the egress completes its pending send, then observes
        // `Lagged` on the next `recv` and resyncs from the ring/log.
        credits.add_permits(10_000);

        // Convergence: collected Event seqs reach the authoritative tail (30).
        let mut seqs = vec![];
        while seqs.last().copied() != Some(30) {
            let m = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
                .await
                .expect("egress stalled before converging")
                .expect("egress sink closed before converging");
            let v: serde_json::Value = serde_json::from_str(msg_text(&m)).unwrap();
            if v["type"] == "event" {
                seqs.push(v["command"]["seq"].as_i64().unwrap());
            }
        }

        // The lag path fired deterministically (the regression guard a larger ring
        // could not provide).
        assert!(
            room.stats.lagged_drops.load(Ordering::Relaxed) > 0,
            "the lag-driven resync path must fire deterministically"
        );
        assert_eq!(*seqs.last().unwrap(), 30);
        let mut sorted = seqs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(seqs, sorted, "no duplicates or reordering after resync");

        drop(etx);
        let _ = egress.await;
    }

    /// The M9c dispatch-layer accumulation: a masked vision payload grows + persists the player's
    /// explored fog and gains a scene-tagged `explored` set; a revisit re-emits without growing; a
    /// GM `mode:"all"` payload is untouched (no fog → no explored).
    #[tokio::test]
    async fn enrich_accumulates_persists_and_emits_explored() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = Uuid::from_u128(1);
        let scene = Uuid::from_u128(10);
        let user = Uuid::from_u128(20);
        let grid = std::collections::HashMap::from([(scene, 100.0)]);

        // A masked payload with a visibility polygon covering a 3×3 cell block in `scene`.
        let mut payload = json!({
            "mode": "masked",
            "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
        });
        enrich_vision_explored(&mut payload, &grid, &repo, world, user, true).await;

        // The payload gained a scene-tagged explored cell set (9 cells × 2 coords).
        let explored = payload["explored"].as_array().unwrap();
        assert_eq!(explored.len(), 1);
        assert_eq!(explored[0]["scene"], json!(scene));
        assert_eq!(explored[0]["cell"], json!(100.0));
        assert_eq!(explored[0]["cells"].as_array().unwrap().len(), 9 * 2);

        // It persisted: a fresh read returns the same 9 cells.
        let stored = crate::scene::explored::ExploredSet::from_bytes(
            &repo.get_explored(scene, user).await.unwrap().unwrap(),
        );
        assert_eq!(stored.len(), 9);

        // A revisit of the same area re-emits the same explored without growing the stored set.
        let mut again = json!({
            "mode": "masked",
            "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
        });
        enrich_vision_explored(&mut again, &grid, &repo, world, user, true).await;
        assert_eq!(
            again["explored"][0]["cells"].as_array().unwrap().len(),
            9 * 2
        );
        assert_eq!(
            crate::scene::explored::ExploredSet::from_bytes(
                &repo.get_explored(scene, user).await.unwrap().unwrap()
            )
            .len(),
            9,
            "revisiting adds no cells"
        );

        // A GM payload (no fog) is left untouched — no explored memory.
        let mut gm = json!({ "mode": "all" });
        enrich_vision_explored(&mut gm, &grid, &repo, world, user, true).await;
        assert_eq!(gm, json!({ "mode": "all" }));
    }

    /// A GM see-as-player view (`accumulate = false`) emits the target's stored explored but is a
    /// read-only observer: it never grows the target's persisted memory from the GM's session.
    #[tokio::test]
    async fn enrich_see_as_player_is_read_only() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = Uuid::from_u128(1);
        let scene = Uuid::from_u128(10);
        let target = Uuid::from_u128(20);
        let grid = std::collections::HashMap::from([(scene, 100.0)]);

        // Seed the target with one explored cell (as if they'd been there).
        let mut seed = crate::scene::explored::ExploredSet::new();
        seed.mark_polygons(
            &[vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0]],
            100.0,
        );
        repo.set_explored(world, scene, target, &seed.to_bytes())
            .await
            .unwrap();

        // The GM views as the target over a polygon covering a 3×3 block (would mark 9 cells if it
        // accumulated). Read-only: emits the stored 1 cell, persists nothing new.
        let mut payload = json!({
            "mode": "masked",
            "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
        });
        enrich_vision_explored(&mut payload, &grid, &repo, world, target, false).await;
        assert_eq!(
            payload["explored"][0]["cells"].as_array().unwrap().len(),
            2, // one stored cell × 2 coords
            "emits only the target's stored explored"
        );
        assert_eq!(
            crate::scene::explored::ExploredSet::from_bytes(
                &repo.get_explored(scene, target).await.unwrap().unwrap()
            )
            .len(),
            1,
            "see-as did not grow the target's persisted memory"
        );
    }

    /// `handle_pathfind` replies to the requesting connection only (one-shot).
    /// GM gets PathResult (no mask). Non-GM in a dark scene (movementRestriction="visible",
    /// env_intensity=0, no placed lights) gets PathError "unreachable" — empty mask blocks all cells.
    #[tokio::test]
    async fn pathfind_handler_gm_ok_nongm_dark_unreachable() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
        use crate::ws::room::RoomRegistry;
        use serde_json::json;

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let author = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", author, 0).await.unwrap();
        let gm_ctx = PermissionContext {
            user_id: author,
            world_role: WorldRole::Gm,
        };

        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;

        let (scene_id, token_id, ws_id) = (
            Uuid::from_u128(0xA001),
            Uuid::from_u128(0xA002),
            Uuid::from_u128(0xA003),
        );

        // World-settings: visible restriction, totally dark (env_intensity=0, no placed lights).
        // A non-GM's visible_cells mask is therefore empty; all non-GM moves are blocked.
        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
        )
        .await
        .unwrap();

        // Scene with a 100-unit grid.
        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
        )
        .await
        .unwrap();

        // Player-owned token at (50,50) = cell (0,0). The player sees nothing (dark scene).
        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
        )
        .await
        .unwrap();

        let rid = Uuid::from_u128(0xF001);

        // GM: unconstrained (no mask) → PathResult for any reachable goal.
        let gm_result = handle_pathfind(
            rid,
            scene_id,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.1,
            &gm_ctx,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(gm_result, ServerMsg::PathResult { .. }),
            "GM should get PathResult; got {gm_result:?}"
        );

        // Non-GM in a dark scene: mask is empty → every cell is out-of-mask → PathError "unreachable".
        // This is the documented fail-closed behaviour: dark scene + visible restriction freezes movement.
        let player_result = handle_pathfind(
            rid,
            scene_id,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.1,
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(player_result, ServerMsg::PathError { ref message, .. } if message == "unreachable"),
            "non-GM in dark scene should be unreachable; got {player_result:?}"
        );
    }

    /// `handle_move_request` executes a move, broadcasts `MoveStream` to the room,
    /// and returns no success frame to the requester. The broadcast carries non-empty
    /// samples terminating at the goal. A rejected move still yields `MoveError` to
    /// the requester only.
    #[tokio::test]
    async fn handle_move_request_broadcasts_move_stream_no_etx_on_success() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
        use crate::ws::protocol::ServerMsg;
        use crate::ws::room::RoomRegistry;

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let author = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", author, 0).await.unwrap();
        let gm_ctx = PermissionContext {
            user_id: author,
            world_role: WorldRole::Gm,
        };

        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;

        let (scene_id, token_id, ws_id) = (
            Uuid::from_u128(0xB001),
            Uuid::from_u128(0xB002),
            Uuid::from_u128(0xB003),
        );

        // World-settings: unrestricted movement so the player token can move freely.
        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
        )
        .await
        .unwrap();

        // Scene with a 100-unit grid.
        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
        )
        .await
        .unwrap();

        // Player-owned token at (50,50). Start = cell-center (0,0); goal = (150,50) = one step right.
        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
        )
        .await
        .unwrap();

        // Subscribe BEFORE issuing the request so the broadcast is not missed.
        let (mut rx, _) = room.subscribe();

        let request_id = Uuid::from_u128(7);
        let expected_goal = [150.0_f64, 50.0_f64];

        // Success: handle_move_request returns None (no etx frame to the requester).
        let result = handle_move_request(
            &room,
            repo.as_ref(),
            &player,
            scene_id,
            token_id,
            vec![[50.0, 50.0], [150.0, 50.0]],
            request_id,
        )
        .await;
        assert!(
            result.is_none(),
            "success path must return None (no etx frame); got {result:?}"
        );

        // The broadcast ring must contain a MoveStream observable on a second subscriber.
        // broadcast_aux sends to existing receivers; rx was subscribed before the call.
        let bcast = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if matches!(*msg, ServerMsg::MoveStream { .. }) {
                            return Some((*msg).clone());
                        }
                        // Skip other frames (e.g. position Event from commit_ops_locked).
                    }
                    Err(_) => return None,
                }
            }
        })
        .await
        .expect("timed out waiting for MoveStream broadcast")
        .expect("receiver closed before MoveStream");

        match bcast {
            ServerMsg::MoveStream {
                request_id: rid,
                token_id: tid,
                mover,
                stop,
                samples,
                mover_vision,
                ..
            } => {
                assert_eq!(rid, request_id, "request_id must be correlated");
                assert_eq!(tid, token_id, "token_id must match");
                assert_eq!(mover, p, "mover must be the player");
                assert_eq!(stop, expected_goal, "stop must equal the goal");
                assert!(!samples.is_empty(), "samples must be non-empty");
                assert!(
                    (samples[0].t_ms - 0.0).abs() < 1e-9,
                    "first sample t_ms must be 0"
                );
                assert_eq!(
                    samples.last().unwrap().pos,
                    expected_goal,
                    "last sample pos must equal stop"
                );
                assert!(
                    mover_vision.is_none(),
                    "mover_vision must be None at this stage"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }

        // Rejection: a move for a non-existent token yields MoveError to etx only.
        let bad_token = Uuid::from_u128(0xDEAD);
        let err_result = handle_move_request(
            &room,
            repo.as_ref(),
            &player,
            scene_id,
            bad_token,
            vec![[50.0, 50.0], [150.0, 50.0]],
            Uuid::from_u128(8),
        )
        .await;
        assert!(
            matches!(err_result, Some(ServerMsg::MoveError { .. })),
            "rejection must return MoveError; got {err_result:?}"
        );
    }

    /// A token-less player (masked + empty polygons) accumulates nothing and emits empty explored
    /// → full fog. No per-scene secret memory is fabricated.
    #[tokio::test]
    async fn enrich_token_less_player_emits_no_explored() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let grid = std::collections::HashMap::new();
        let mut payload = json!({ "mode": "masked", "polygons": [] });
        enrich_vision_explored(
            &mut payload,
            &grid,
            &repo,
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            true,
        )
        .await;
        assert_eq!(payload["explored"].as_array().unwrap().len(), 0);
    }

    // ─── clip_move_stream: per-recipient secrecy boundary tests ───────────────

    /// Shared setup for `clip_move_stream` integration tests: creates an in-memory world, a GM
    /// user, an observer player user, one scene, and optionally an observer token + a wall doc.
    /// Returns `(room, gm_ctx, observer_ctx, scene_id)`.
    ///
    /// world-settings are omitted — `player_vision_polygons` only needs tokens + walls.
    async fn setup_clip_room(
        obs_token_pos: Option<(f64, f64)>,
        wall_system: Option<serde_json::Value>,
        wall_gm_only: bool,
    ) -> (
        Arc<crate::ws::room::Room>,
        PermissionContext,
        PermissionContext,
        Uuid,
    ) {
        use crate::auth::role::ServerRole;
        use crate::data::command::Operation;
        use crate::data::document::{DocRole, WorldRole};
        use crate::ws::room::RoomRegistry;

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };

        let obs = repo
            .create_user("obs", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, obs, WorldRole::Player)
            .await
            .unwrap();
        let obs_ctx = PermissionContext {
            user_id: obs,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;

        let scene_id = Uuid::from_u128(0xE001);
        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(gm);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
        )
        .await
        .unwrap();

        if let Some(pos) = obs_token_pos {
            let token_id = Uuid::from_u128(0xE002);
            let mut tok = wdoc(world.id, token_id, "token");
            tok.parent_id = Some(scene_id);
            tok.owner = Some(obs);
            tok.permissions.users.insert(obs, DocRole::Owner);
            tok.system = json!({ "x": pos.0, "y": pos.1 });
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![Operation::Create { doc: tok }],
                0,
            )
            .await
            .unwrap();
        }

        if let Some(ws) = wall_system {
            let wall_id = Uuid::from_u128(0xE003);
            let mut wall = wdoc(world.id, wall_id, "wall");
            wall.parent_id = Some(scene_id);
            wall.owner = Some(gm);
            wall.system = ws;
            if wall_gm_only {
                // gm_only wall: DocRole::None means players cannot read the doc;
                // sight_walls uses the FULL wall set regardless (permission-blind).
                wall.permissions.default = DocRole::None;
            }
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![Operation::Create { doc: wall }],
                0,
            )
            .await
            .unwrap();
        }

        (room, gm_ctx, obs_ctx, scene_id)
    }

    /// The mover (ctx.user_id == frame.mover) receives their own full frame
    /// unchanged — all samples and `mover_vision` are forwarded verbatim.
    #[tokio::test]
    async fn clip_mover_receives_full_frame() {
        use crate::data::document::WorldRole;
        use crate::ws::protocol::{PosSample, VisionSample};

        let (room, _, _, scene_id) = setup_clip_room(None, None, false).await;

        let mover_id = Uuid::from_u128(0xAABB);
        // ctx.user_id == mover → mover branch fires before GM / observer branches.
        let ctx = PermissionContext {
            user_id: mover_id,
            world_role: WorldRole::Player,
        };

        let samples = vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [150.0, 50.0],
            },
        ];
        let mv = Some(vec![VisionSample {
            t_ms: 0.0,
            polygons: vec![vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]],
        }]);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: 400.0,
            stop: [150.0, 50.0],
            samples: samples.clone(),
            mover_vision: mv.clone(),
        };

        let result = clip_move_stream(&frame, &ctx, &room).await;

        assert!(result.is_some(), "mover must receive a frame");
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv_out,
                ..
            } => {
                assert_eq!(s, samples, "mover receives all samples unchanged");
                assert_eq!(mv_out, mv, "mover receives mover_vision unchanged");
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// An observer with no token in the scene has empty vision polygons → every sample is
    /// outside their vision → the frame is suppressed entirely (None, not empty-samples Some).
    #[tokio::test]
    async fn clip_observer_no_token_suppressed() {
        use crate::ws::protocol::PosSample;

        // No observer token in the scene — player_vision_polygons returns empty.
        let (room, _, obs_ctx, scene_id) = setup_clip_room(None, None, false).await;

        let mover_id = Uuid::from_u128(0xAABB);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: 600.0,
            stop: [250.0, 50.0],
            samples: vec![
                PosSample {
                    t_ms: 0.0,
                    pos: [50.0, 50.0],
                },
                PosSample {
                    t_ms: 200.0,
                    pos: [150.0, 50.0],
                },
                PosSample {
                    t_ms: 400.0,
                    pos: [250.0, 50.0],
                },
            ],
            mover_vision: None,
        };

        let result = clip_move_stream(&frame, &obs_ctx, &room).await;

        assert!(
            result.is_none(),
            "observer with no token must receive no frame (suppressed); got {result:?}"
        );
    }

    /// An observer whose token is on the near side of a `blocksSight` wall sees only samples
    /// on their side. The clipped frame carries those samples with `mover_vision = None`.
    ///
    /// Setup: observer token at (50,50); vertical wall at x=100. Samples at (50,50), (150,50),
    /// (250,50). Only (50,50) is on the near side — the other two are occluded.
    #[tokio::test]
    async fn clip_observer_sees_near_side_prefix() {
        use crate::ws::protocol::PosSample;

        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, _, obs_ctx, scene_id) =
            setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

        let mover_id = Uuid::from_u128(0xAABB);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: 600.0,
            stop: [250.0, 50.0],
            samples: vec![
                PosSample {
                    t_ms: 0.0,
                    pos: [50.0, 50.0], // near side — observer can see this
                },
                PosSample {
                    t_ms: 200.0,
                    pos: [150.0, 50.0], // behind wall — occluded
                },
                PosSample {
                    t_ms: 400.0,
                    pos: [250.0, 50.0], // further behind wall — occluded
                },
            ],
            mover_vision: None,
        };

        let result = clip_move_stream(&frame, &obs_ctx, &room).await;

        assert!(
            result.is_some(),
            "partial-visibility observer must receive a clipped frame"
        );
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv,
                stop: out_stop,
                duration_ms: out_duration_ms,
                ..
            } => {
                assert_eq!(
                    s.len(),
                    1,
                    "only one sample (near side) should be visible; got {} samples: {s:?}",
                    s.len()
                );
                assert_eq!(
                    s[0].pos,
                    [50.0_f64, 50.0_f64],
                    "visible sample must be (50,50)"
                );
                assert_eq!(mv, None, "mover_vision must be None for observers");
                // Critical 2 regression: stop and duration_ms must be clipped to
                // the last visible sample, NOT the true goal/full travel distance.
                assert_eq!(
                    out_stop,
                    [50.0_f64, 50.0_f64],
                    "stop must be clipped to last visible sample pos, not the true goal"
                );
                assert!(
                    (out_duration_ms - 0.0_f64).abs() < 1e-9,
                    "duration_ms must be clipped to last visible sample t_ms (0 ms), got {out_duration_ms}"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// A `gm_only` (`DocRole::None`) `blocksSight` wall bounds the observer's authoritative
    /// vision identically to a normal wall — `sight_walls` is permission-blind (full wall set,
    /// M9b invariant). When the mover's entire path lies behind the secret wall, the frame is
    /// fully suppressed: the observer receives zero `MoveStream` frames, not an empty-sample one.
    #[tokio::test]
    async fn clip_gm_only_wall_suppresses_observer() {
        use crate::ws::protocol::PosSample;

        // gm_only wall at x=100; observer token at (50,50) cannot see x>100.
        // All mover samples are beyond the wall → every sample is occluded → suppress.
        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, _, obs_ctx, scene_id) =
            setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), true /* gm_only */).await;

        let mover_id = Uuid::from_u128(0xAABB);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: 400.0,
            stop: [250.0, 50.0],
            samples: vec![
                PosSample {
                    t_ms: 0.0,
                    pos: [150.0, 50.0], // behind gm_only wall — occluded
                },
                PosSample {
                    t_ms: 200.0,
                    pos: [250.0, 50.0], // further behind — also occluded
                },
            ],
            mover_vision: None,
        };

        let result = clip_move_stream(&frame, &obs_ctx, &room).await;

        // Must be None (fully suppressed), NOT Some(MoveStream { samples: [], .. }).
        // The secrecy invariant: zero frames sent, never an empty-samples frame.
        assert!(
            result.is_none(),
            "observer behind gm_only wall must receive zero MoveStream frames (None, not \
             Some(empty)); got {result:?}"
        );
    }

    /// A GM who is NOT the mover receives ALL position samples regardless of LOS, with
    /// `mover_vision` nulled and the full `stop`/`duration_ms` intact.
    ///
    /// Invariants verified:
    /// - GM branch fires before observer branch (ctx.user_id != mover, but Gm role).
    /// - All samples pass through unfiltered.
    /// - `mover_vision` is never forwarded to anyone but the mover.
    /// - `stop` and `duration_ms` are the full values (no clip for GM).
    #[tokio::test]
    async fn clip_gm_receives_all_samples_mover_vision_nulled() {
        use crate::data::document::WorldRole;
        use crate::ws::protocol::{PosSample, VisionSample};

        // Wall at x=100; the mover's samples cross to the far side, but a GM sees everything.
        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, gm_ctx, _, scene_id) = setup_clip_room(None, Some(wall_sys), false).await;

        // GM is NOT the mover.
        let mover_id = Uuid::from_u128(0xAABB);
        assert_ne!(
            gm_ctx.user_id, mover_id,
            "GM must not be the mover in this test"
        );
        assert_eq!(gm_ctx.world_role, WorldRole::Gm);

        let samples = vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [150.0, 50.0],
            }, // behind wall — still visible to GM
            PosSample {
                t_ms: 400.0,
                pos: [250.0, 50.0],
            },
        ];
        let true_stop = [250.0_f64, 50.0_f64];
        let true_duration_ms = 600.0_f64;
        let mv = Some(vec![VisionSample {
            t_ms: 0.0,
            polygons: vec![vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]],
        }]);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: true_duration_ms,
            stop: true_stop,
            samples: samples.clone(),
            mover_vision: mv,
        };

        let result = clip_move_stream(&frame, &gm_ctx, &room).await;

        assert!(result.is_some(), "GM must receive a frame");
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv_out,
                stop: out_stop,
                duration_ms: out_duration_ms,
                ..
            } => {
                assert_eq!(s, samples, "GM receives all samples unchanged");
                assert_eq!(mv_out, None, "mover_vision must be nulled for GM observers");
                assert_eq!(out_stop, true_stop, "GM receives the full stop (no clip)");
                assert!(
                    (out_duration_ms - true_duration_ms).abs() < 1e-9,
                    "GM receives the full duration_ms (no clip)"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }
}
