//! WebSocket upgrade and per-connection ingress/egress tasks.
//!
//! All socket writes happen in the egress task (it owns the sink). The ingress
//! task parses client frames and forwards intents to egress over an mpsc
//! channel, or publishes directly to the room. The egress task multiplexes the
//! lossy broadcast stream (with a sequence guard + lag-driven resync) and the
//! ingress intent channel onto the one socket.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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
use crate::data::command::WriteOrigin;
use crate::data::document::WorldCapDefaults;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::http::AppState;
use crate::ws::protocol::{ClientMsg, RejectReason, ServerMsg, WsErrorCode};
use crate::ws::room::Room;
use crate::ws::time::now_millis;

/// Query parameters of the `/ws` upgrade request.
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// The world the connection joins.
    pub world: Uuid,
}

/// Intents the ingress task hands to the egress task (which owns the sink).
enum Egress {
    /// Deliver a ready frame to this connection.
    Frame(Arc<ServerMsg>),
    /// Send a time-calibration reply.
    TimePong {
        /// Echo of the ping's client send time.
        client_t0: i64,
        /// Server wall-clock at reply, ms.
        server_t: i64,
    },
    /// Run a resync replay from this seq.
    Resync(i64),
    /// Register a live search subscription (the egress task owns the registry).
    Subscribe {
        /// Subscription correlation token.
        request_id: Uuid,
        /// The live query text.
        query: String,
        /// Top-N size.
        limit: u32,
    },
    /// Cancel a live search subscription.
    Unsubscribe {
        /// The subscription to cancel.
        request_id: Uuid,
    },
    /// Register a derived scene-channel subscription (egress-owned). `as_user` (GM-only
    /// see-as-player) is authorized + resolved in the egress handler.
    SceneSubscribe {
        /// Subscription correlation token.
        request_id: Uuid,
        /// Channel name.
        channel: String,
        /// GM-only see-as-player target (authorized in the egress handler).
        as_user: Option<Uuid>,
    },
    /// Cancel a derived scene-channel subscription.
    SceneUnsubscribe {
        /// The subscription to cancel.
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
/// Per-user chat flood budget: messages allowed per trailing 60s window.
const MESSAGE_RATE_PER_MIN: usize = 30;

/// A live search subscription's stored state.
struct Sub {
    /// The live query text.
    query: String,
    /// Top-N size.
    limit: u32,
    /// Last delivered result identity, in rank order. Used to suppress a push
    /// when re-evaluation yields an identical top-N.
    fingerprint: Vec<(Uuid, u64, i64)>,
}

/// A derived scene-channel subscription's stored state. `fingerprint` is the
/// last delivered payload; a re-eval pushes only when it changes. `view_ctx` is the effective
/// context the channel is computed for: the connection's own ctx, or — for a GM see-as-player
/// subscription — the server-resolved target player's context.
struct SceneSub {
    /// Channel name.
    channel: String,
    /// Last delivered payload; a re-eval pushes only on change.
    fingerprint: Option<serde_json::Value>,
    /// The context the channel is computed for (own, or GM see-as target).
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
    room: &Room,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    msg: &ServerMsg,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    let out = match msg {
        ServerMsg::Event { command, intent_id } => {
            // Loads complete BEFORE the guard: no lock across await. The guard is
            // held only around the synchronous core — the same short-read-guard
            // discipline as clip_move_stream.
            let current = crate::data::permission::load_update_docs(repo, command).await;
            let filtered = {
                let ecs = room.scene().read().await;
                crate::data::permission::filter_command(
                    command,
                    ctx,
                    world_defaults,
                    &current,
                    |id| ecs.actor(id),
                )
            };
            ServerMsg::Event {
                command: filtered,
                intent_id: *intent_id,
            }
        }
        other => {
            // MoveStream carries per-recipient position geometry and MUST be clipped
            // through `clip_move_stream` in the egress loop, never passed through here
            // unredacted. This guard catches a future routing regression at test time.
            debug_assert!(
                !matches!(other, ServerMsg::MoveStream { .. }),
                "MoveStream must be clipped per-recipient in egress_loop, not sent via send_filtered"
            );
            other.clone()
        }
    };
    sink.send(text(&out)).await.map_err(|_| ())
}

/// One connection's lifetime: splits the socket into an ingress task (parses
/// frames, applies intents through the one write path) and an egress task
/// (owns the sink + subscription registries), joined until either ends.
///
/// # Examples
///
/// ```text
/// // Spawned by the /ws upgrade handler once auth + world membership pass.
/// ```
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
    let modules_dir = state.config.modules_path();
    let mut egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        egress_room,
        egress_repo,
        ctx,
        current_seq,
        modules_dir,
    ));

    // Ingress: parse client frames, forward intents to egress / publish.
    // Per-user ping budget (shared across this user's connections; survives reconnect).
    let ping_rate = state.ws.ping_rate.clone();
    // Per-user chat flood budget (shared across this user's connections).
    let message_rate = state.ws.message_rate.clone();
    // Link-preview fetch client/cache/budget (shared across all connections
    // and worlds — a preview's target and cached outcome are world-independent).
    let preview_client = state.ws.link_preview_client.clone();
    let preview_cache = state.ws.link_preview_cache.clone();
    let preview_rate = state.ws.preview_rate.clone();
    loop {
        tokio::select! {
            _ = &mut egress => break,
            frame = stream.next() => {
                let Some(Ok(frame)) = frame else { break };
                match frame {
                    Message::Text(t) => match serde_json::from_str::<ClientMsg>(t.as_str()) {
                        Ok(ClientMsg::Intent { intent_id, ops }) => {
                            // Messages are server-authored via SendMessage only;
                            // a client-authored message op is always rejected here,
                            // never reaching apply_intent.
                            if crate::chat::ops_target_message(&ops) {
                                let _ = etx
                                    .send(Egress::Frame(Arc::new(ServerMsg::Reject {
                                        intent_id,
                                        reason: RejectReason::Forbidden,
                                    })))
                                    .await;
                                continue;
                            }
                            // Success is confirmed by the broadcast echo of the
                            // authored Event; only a rejection is sent directly.
                            match room.publish(repo.as_ref(), &ctx, ops, now_millis(), WriteOrigin::Client).await {
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
                            // coordinates are not validated. Guard order: cheap rate check
                            // first, then the authz lookup (one doc read per admitted ping,
                            // bounded at 30/min/user). Over-budget and unauthorized pings both
                            // drop silently — no error frame, so a non-reader never learns
                            // whether `scene` exists.
                            if ping_rate.check(user_id, now_millis(), 30)
                                && scene_ping_permitted(scene, &ctx, world_id, repo.as_ref())
                                    .await
                            {
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
                        Ok(ClientMsg::SendMessage { request_id, channel, content, actor_owner, audience }) => {
                            // Server-authoritative chat ingest: flood-limit, validate, CONSTRUCT
                            // the message doc, and publish. Success is confirmed by the broadcast
                            // echo of the authored Event (like Intent). On rejection, a
                            // `ChatError` correlated by `request_id` is sent to the sender ONLY
                            // (never broadcast) so the failure is surfaced instead of vanishing;
                            // `message` is the classified, no-leak `Display` text.
                            if let Err(e) = crate::chat::handle_send_message(
                                &room,
                                repo.as_ref(),
                                &ctx,
                                &message_rate,
                                crate::chat::LinkPreviewDeps { client: &preview_client, cache: &preview_cache, rate: &preview_rate },
                                channel,
                                content,
                                actor_owner,
                                audience,
                                now_millis(),
                                MESSAGE_RATE_PER_MIN,
                            )
                            .await
                            {
                                tracing::debug!(world = %world_id, user = %user_id, ?e, "message rejected");
                                if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                    request_id,
                                    message: e.to_string(),
                                }))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::EditMessage { request_id, message_id, content }) => {
                            // Same confirm-by-broadcast-echo shape as SendMessage; a rejection is
                            // surfaced to the sender only via a `request_id`-correlated `ChatError`.
                            if let Err(e) = crate::chat::handle_edit_message(
                                &room,
                                repo.as_ref(),
                                &ctx,
                                &message_rate,
                                crate::chat::LinkPreviewDeps { client: &preview_client, cache: &preview_cache, rate: &preview_rate },
                                message_id,
                                content,
                                now_millis(),
                                MESSAGE_RATE_PER_MIN,
                            )
                            .await
                            {
                                tracing::debug!(world = %world_id, user = %user_id, ?e, "edit rejected");
                                if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                    request_id,
                                    message: e.to_string(),
                                }))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::DeleteMessage { request_id, message_id }) => {
                            // Same confirm-by-broadcast-echo shape as SendMessage/EditMessage; a
                            // rejection is surfaced to the sender only via a correlated `ChatError`.
                            if let Err(e) = crate::chat::handle_delete_message(
                                &room,
                                repo.as_ref(),
                                &ctx,
                                &message_rate,
                                message_id,
                                now_millis(),
                                MESSAGE_RATE_PER_MIN,
                            )
                            .await
                            {
                                tracing::debug!(world = %world_id, user = %user_id, ?e, "delete rejected");
                                if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                    request_id,
                                    message: e.to_string(),
                                }))).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(ClientMsg::Pathfind { request_id, scene, start, waypoints, footprint_radius, token }) => {
                            // One-shot pathfinding: resolve GM status, fetch explored off the lock for
                            // non-GM Revealed, call SceneEcs::pathfind, reply to this connection only.
                            // INVARIANT (one-shot-to-requester): reply goes to etx only, never broadcast.
                            let frame = handle_pathfind(
                                request_id, scene, start, waypoints, footprint_radius, token,
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

/// Whether `ctx` may ping into `scene`: the doc must exist, be a `scene`, belong to THIS world,
/// and grant the sender `cap::READ`. Admits a token-less spectator (READ on the scene is enough
/// — deliberately weaker than `handle_pathfind`'s controls-a-token gate, which selects server
/// state; ping selects none). Denial is a SILENT drop at the call site: any error frame or
/// behavior split would leak scene existence to a non-reader.
async fn scene_ping_permitted(
    scene: Uuid,
    ctx: &crate::data::membership::PermissionContext,
    world_id: Uuid,
    repo: &dyn crate::data::repository::Repository,
) -> bool {
    let Ok(Some(doc)) = repo.get_document(scene).await else {
        return false;
    };
    if doc.doc_type != "scene" {
        return false;
    }
    // World scope: a scene doc from another world is refused even for a member of both (the
    // relay stamps THIS room).
    if crate::data::document::world_of(&doc) != Some(world_id) {
        return false;
    }
    let Ok(defaults) = repo.world_cap_defaults(world_id).await else {
        return false;
    };
    // A scene doc never carries an actor link, so the no-join resolution is
    // exact — and fails closed if a non-scene doc ever reached here.
    let access = crate::data::permission::resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &doc,
        &defaults.grants_for(&doc.doc_type),
        crate::data::permission::effective_owner(&doc, None),
    );
    access.has(crate::data::permission::cap::READ)
}

/// Resolve and execute a one-shot grid pathfind request.
///
/// INVARIANT (no-lock-across-await): the scene read guard is taken twice — once to read
/// `movement_restriction` (then dropped), and once to call `pathfind` (then dropped again) —
/// so `get_explored` can be awaited between them without holding the lock.
/// INVARIANT (one-shot-to-requester): the reply is placed directly on `etx`; it is never
/// broadcast to the room.
/// INVARIANT (scene presence): a non-GM requester must control a token in the named scene. A
/// `Pathfind` frame MAY name a token (`token`), but that is a footprint source, not a presence
/// proof — it is separately authorized (owned + parented to this scene) and never substitutes for
/// this scan. Without the scan a player can route-preview inside a scene they have never entered:
/// an `unrestricted` scene has no visibility mask to fail closed on, and the returned polyline
/// discloses that scene's `blocksMove` wall layout.
#[allow(clippy::too_many_arguments)]
async fn handle_pathfind(
    request_id: Uuid,
    scene: Uuid,
    start: (f64, f64),
    waypoints: Vec<(f64, f64)>,
    footprint_radius: f64,
    token: Option<Uuid>,
    ctx: &crate::data::membership::PermissionContext,
    room: &crate::ws::room::Room,
    repo: &dyn crate::data::repository::Repository,
) -> ServerMsg {
    let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
    // Step 0: non-GM presence gate, ahead of any routing work. `user_owns_token_in_scene` is a
    // document scan routed through `token_effective_owner`, so ownership is the same rule the
    // write-authz and vision paths enforce (per-token override, else the linked actor's owner) —
    // and it costs no raycast, which matters because `Pathfind` is unrate-limited and fires
    // repeatedly during a drag preview. The reply is the same generic `PathError` an out-of-mask
    // route gets: it discloses nothing about whether the scene exists, is walled, or is merely
    // unreachable.
    //
    // Deliberate asymmetry — do NOT "fix" it by forking a looser ownership test: this gate keys
    // on effective OWNERSHIP, while the visibility mask additionally unions observer-tier tokens
    // when `observerVision` is on. A user whose only vision in a scene is observer-tier therefore
    // has a mask there but is refused a route preview. That is the fail-closed direction (a route
    // preview is a precursor to moving a token, which observer tier does not grant), and matching
    // the mask's wider source here would hand route previews — and the wall geometry they
    // disclose — to a user who controls nothing in the scene. The below authorization of a named
    // `token` keys on the SAME effective-ownership rule, for the same reason: it is a footprint
    // source, not a wider presence grant.
    if !is_gm {
        let present = {
            let s = room.scene().read().await;
            s.user_owns_token_in_scene(ctx.user_id, scene)
        };
        if !present {
            return ServerMsg::PathError {
                request_id,
                message: "unreachable".to_string(),
            };
        }
    }
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
    // Step 3: take a fresh read guard to authorize/derive a named token's footprint (if any) and
    // call pathfind — folded into the SAME guard so the read count is unchanged.
    //
    // The named token is authorized before it is used: effective ownership (the same
    // `token_effective_owner` rule the presence gate and write-authz use — never a forked, looser
    // test) AND membership in the named scene. A caller-supplied token id that skipped either check
    // would be a size oracle, and a cross-scene id would source a footprint from a scene the
    // requester has no presence in. Failure returns the SAME generic PathError an unreachable route
    // gets, disclosing nothing about the token's existence. A GM is exempt from the ownership half
    // (they control the scene) but not from the scene-membership half.
    let s = room.scene().read().await;
    let footprint_radius = match token {
        Some(t) => {
            let derived = match s.token_scene_and_effective_owner(t) {
                Some((t_scene, _)) if t_scene != scene => None,
                Some((_, owner)) if !is_gm && owner != Some(ctx.user_id) => None,
                Some(_) => s.resolve_token_footprint(t),
                None => None,
            };
            match derived {
                Some(r) => r,
                None => {
                    return ServerMsg::PathError {
                        request_id,
                        message: "unreachable".to_string(),
                    };
                }
            }
        }
        None => footprint_radius,
    };
    match s.pathfind(
        ctx.user_id,
        scene,
        start,
        &waypoints,
        footprint_radius,
        is_gm,
        explored.as_ref(),
    ) {
        Ok(outcome) => ServerMsg::PathResult {
            request_id,
            path: outcome.path,
            cost: outcome.cost,
            arrested: outcome.arrested,
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
                // The scene the token actually lives in (`execute_move` derives it from the
                // token, never from the request), so the per-recipient egress clip and the
                // client's viewed-scene filter both key on the committed scene.
                scene: exec.scene,
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
                // Broadcast in-process carries the full authoritative cost; `clip_move_stream`
                // nulls it per recipient at egress for a clipped observer (secrecy: see
                // `ServerMsg::MoveStream.cost` doc).
                cost: Some(exec.cost),
                // Same trusted-only treatment as `cost`: full value in-process, nulled per
                // recipient at egress for a clipped observer.
                truncated: Some(exec.truncated),
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
/// see-as-player view: it is a read-only observer that emits the target's stored explored
/// but must NOT grow the target's memory from the GM's session.
async fn enrich_vision_explored(
    payload: &mut serde_json::Value,
    grid: &std::collections::HashMap<Uuid, f64>,
    grid_shapes: &std::collections::HashMap<
        Uuid,
        Box<dyn crate::scene::grid_shape::GridShape + Send + Sync>,
    >,
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
        // Index this scene's explored fog through its own resolved grid shape (hex axial on a hex
        // scene, byte-identical square math otherwise) so the accumulated cells compose with the
        // `Revealed` gate's hex `line_traversal` move-cells. A scene absent from either map has no
        // live scene document — skip it (fail closed: the client masks everything outside
        // `polygons`, so a skipped scene simply contributes no explored). Never synthesize a grid
        // no scene declared.
        let Some(cell) = grid.get(&scene).copied() else {
            continue;
        };
        // `+ Send + Sync` so the borrow may live across the `get_explored` await below (the egress
        // task future must be `Send`); coerces to `&dyn GridShape` at the `mark_polygons` call.
        let Some(shape) = grid_shapes
            .get(&scene)
            .map(|b| b.as_ref() as &(dyn crate::scene::grid_shape::GridShape + Send + Sync))
        else {
            continue;
        };
        let mut set = match repo.get_explored(scene, user).await {
            Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob),
            _ => crate::scene::explored::ExploredSet::new(),
        };
        if accumulate && set.mark_polygons(&scene_polys, shape, cell) > 0 {
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
///   samples + `mover_vision` + `cost`).
/// - **GM** (world role) with NO applicable see-as: all samples and the true `cost`
///   forwarded (trusted, full information), `mover_vision` nulled, full `stop` and
///   `duration_ms` preserved.
/// - **GM with an active see-as-player preview** (`see_as = Some(target)`) whose target has
///   a vision source in the move's scene: the GM's OWN view is narrowed to exactly what the
///   previewed target would see, via the SAME clip path a real observer gets (keyed on the
///   target's vision, not the GM's). A see-as whose target has no source in the move's scene
///   does not apply → full GM stream. This branch can only ever NARROW the GM's own view.
/// - **Observer**: only samples whose `pos` lies within the recipient's authoritative
///   vision polygons are forwarded; `mover_vision` AND `cost` nulled; fully-occluded →
///   `None`. `stop` and `duration_ms` are clipped to the LAST VISIBLE sample — the true
///   final position and full travel distance are not disclosed.
///
/// INVARIANT (mover_vision-isolation): `mover_vision` reaches only the mover's socket.
/// INVARIANT (no-cost-leak): the true `cost` reaches only the mover and GM sockets; a
///   clipped observer receives `None` (mirrors mover_vision-isolation) so hidden
///   (`gm_only`) region terrain cannot be inferred by diffing visible distance vs. cost.
/// INVARIANT (fail-closed): no derivable vision → empty clip → suppress.
/// INVARIANT (no-stale-cache): observer vision is always read from the authoritative ECS,
///   never from a rendering-cache fingerprint (a stale wider polygon would admit a
///   now-hidden sample).
/// INVARIANT (no-lock-across-await): the ECS read lock (if taken) is dropped inside
///   `observer_vision_polys_for_scene` before this function returns.
async fn clip_move_stream(
    msg: &ServerMsg,
    ctx: &PermissionContext,
    see_as: Option<PermissionContext>,
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
        cost,
        truncated,
    } = msg
    else {
        return None;
    };

    // Mover receives their own stream unchanged (all samples + mover_vision). Keyed on the
    // REAL connection user_id, never the see-as target: a GM previewing as someone else is
    // not "the mover" unless the GM's own token is what moves.
    if ctx.user_id == *mover {
        return Some(msg.clone());
    }

    // The full, unclipped GM stream: all position samples pass, mover sightlines and the
    // authoritative cost are trusted-full. This is the GM's default (they are authorized to
    // see everything) and the fallback whenever no see-as clip applies.
    let full_gm_stream = || ServerMsg::MoveStream {
        request_id: *request_id,
        token_id: *token_id,
        mover: *mover,
        scene: *scene,
        start_server_ms: *start_server_ms,
        duration_ms: *duration_ms,
        stop: *stop,
        samples: samples.clone(),
        mover_vision: None,
        cost: *cost,
        truncated: *truncated,
    };

    // Choose whose authoritative vision this recipient's samples are clipped against — or
    // return the full GM stream when no clip applies.
    //
    // INVARIANT (see-as-narrowing-only): the see-as branch is reached ONLY for a GM
    //   (`world_role == Gm`); the observer and mover branches are structurally untouched by
    //   `see_as`, so threading a see-as target can NEVER widen what a non-GM recipient
    //   receives. Every see-as outcome (full stream, clipped, or suppressed) is `<=` what the
    //   plain-GM fallthrough would disclose.
    // INVARIANT (see-as-server-resolved): `see_as` carries the SERVER-RESOLVED target context
    //   the caller read from this connection's own `scene_subs` (populated only by the
    //   `SceneSubscribe` handler, which gates `as_user` to a GM and resolves the target role
    //   via `member_role`). It is never client-trusted geometry.
    // INVARIANT (see-as-scene-exact): the target's vision is computed for the move's EXACT
    //   `scene` via `observer_vision_polys_for_scene`, which filters `player_vision_polygons`
    //   by that scene id. A see-as whose target has NO vision source in the move's scene
    //   (e.g. their token is in a different scene) yields zero polygons → the see-as does not
    //   apply → the GM keeps the full stream. A target WITH a source in the scene but no
    //   visible sample is suppressed, exactly like a real observer.
    let clip_polys: Vec<Vec<crate::scene::vision::P>> =
        if ctx.world_role == crate::data::document::WorldRole::Gm {
            match see_as {
                Some(target) => {
                    let polys = observer_vision_polys_for_scene(target.user_id, *scene, room).await;
                    if polys.is_empty() {
                        // See-as target has no vision source in this scene → not applicable.
                        return Some(full_gm_stream());
                    }
                    polys
                }
                None => return Some(full_gm_stream()),
            }
        } else {
            // Observer: clip to samples within their OWN authoritative vision.
            observer_vision_polys_for_scene(ctx.user_id, *scene, room).await
        };

    // Shared clip tail (a real observer, or a GM whose active see-as applies to this scene):
    // keep only samples inside `clip_polys`. The ECS read is dropped inside
    // `observer_vision_polys_for_scene` before this point, so no lock crosses the `sink.send`
    // await in the caller.
    let polys = clip_polys;
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
    // INVARIANT: `visible` is non-empty here (the is_empty guard above returns None).
    // The unwrap_or fallbacks below are unreachable; the assert makes the secrecy
    // invariant machine-checked so a future refactor of the guard cannot silently
    // fall back to the full unclipped stop/duration.
    debug_assert!(
        !visible.is_empty(),
        "clip invariant: visible must be non-empty (else the move is suppressed)"
    );
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
        // INVARIANT (no-cost-leak): the authoritative `cost` may include secret (`gm_only`)
        // region terrain the observer's clipped `samples` never reveal; disclosing it would
        // let an observer detect/estimate hidden terrain by comparing the visible portion of
        // the move against the reported total. Mirrors `mover_vision`'s null-for-observers
        // treatment above.
        // `cost` is a whole-move scalar: Some for mover/GM, None for a clipped observer —
        // engine-agnostic (grid or continuous), because a continuous weighted cost may
        // reflect gm_only terrain.
        cost: None,
        // INVARIANT (no-truncation-leak): same whole-move-scalar rule as `cost`. The
        // observer's `samples` and `stop` are already clipped to what they witnessed, so a
        // truthful `truncated` would answer a question their clipped view cannot: whether
        // anything stopped the token BEYOND their vision. Disclosing it reveals the presence
        // of a wall or a `gm_only` region they cannot see — and a region-arrest on the final
        // step is invisible to geometry, so this flag is the ONLY channel carrying it.
        truncated: None,
    })
}

/// Union `world_reqs` (GM-authored, unchanged) with the `requirements`
/// declared by each of the world's currently ENABLED installed modules —
/// enabling a module publishes its manifest requirements through the
/// capability machinery. Non-destructive: `world_cap_requirements` itself is
/// NEVER mutated by enable/disable; this union is recomputed fresh on every
/// `Welcome`, so a mid-session enable/disable takes effect on the affected
/// world's next (re)connect, exactly like a `world_cap_requirements` edit
/// already does today. Re-checks `engine_compat_ok` per enabled module (not
/// just at enable time) so a module that has gone incompatible since being
/// enabled (server downgrade, on-disk manifest edit) stops contributing.
///
/// ADVISORY ONLY: the returned union is the client's advisory copy for
/// showing/hiding write controls (the client's `canEdit`). It is
/// NOT the server-side write-enforcement input — `apply_intent`
/// consults only the GM-authored `world_cap_requirements`
/// record, never a module's declared `requirements`.
async fn welcome_capability_requirements(
    repo: &dyn Repository,
    world_id: Uuid,
    modules_dir: &std::path::Path,
) -> Vec<crate::data::document::CapabilityRequirement> {
    // Keyed by path_prefix so a GM-authored requirement and a module-declared
    // requirement on the same prefix union their caps into one entry instead
    // of the client seeing two separate entries for the same prefix.
    let mut by_prefix: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let world_reqs = match repo.world_cap_requirements(world_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "capability requirements unreadable; sending empty");
            Vec::new()
        }
    };
    for r in world_reqs {
        by_prefix.entry(r.path_prefix).or_default().extend(r.caps);
    }
    let enabled = match repo.world_enabled_modules(world_id).await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "enabled modules unreadable; skipping module-published requirements");
            Vec::new()
        }
    };
    if !enabled.is_empty() {
        // Blocking std::fs I/O; run off the async worker on every WS-connect
        // Welcome path, matching the spawn_blocking convention in `hash_password_async`.
        // A panicked scan (JoinError) degrades to an empty Vec, matching the
        // missing-modules_dir behavior already in scan_installed_modules.
        let dir = modules_dir.to_path_buf();
        let installed =
            tokio::task::spawn_blocking(move || crate::modules::scan_installed_modules(&dir))
                .await
                .unwrap_or_default();
        for id in &enabled {
            // Re-check engine-compat here (not just at enable time): a module
            // enabled while compatible can go stale after a server downgrade
            // or an on-disk manifest edit. Engine-compat is enforced at BOTH
            // enable and load — a continuous property, not a one-time gate —
            // so a now-incompatible enabled module must not publish requirements.
            if let Some(m) = installed
                .iter()
                .find(|m| &m.id == id && crate::modules::engine_compat_ok(m))
            {
                for r in &m.requirements {
                    by_prefix
                        .entry(r.path_prefix.clone())
                        .or_default()
                        .extend(r.caps.iter().cloned());
                }
            }
        }
    }
    by_prefix
        .into_iter()
        .map(
            |(path_prefix, caps)| crate::data::document::CapabilityRequirement {
                path_prefix,
                caps,
            },
        )
        .collect()
}

/// The egress half: fans room broadcasts into this connection with
/// per-recipient filtering, serves resyncs/time-pongs, and owns the live
/// search + scene-channel subscription registries (re-evaluated on events,
/// debounced, fingerprint-suppressed).
///
/// # Examples
///
/// ```text
/// // One egress_loop per connection; it exits when the socket or room closes.
/// ```
#[allow(clippy::too_many_arguments)]
async fn egress_loop<S>(
    mut sink: S,
    mut rx: tokio::sync::broadcast::Receiver<Arc<ServerMsg>>,
    mut erx: mpsc::Receiver<Egress>,
    room: Arc<Room>,
    repo: Arc<SqliteRepository>,
    ctx: PermissionContext,
    current_seq: i64,
    modules_dir: std::path::PathBuf,
) where
    S: Sink<Message> + Unpin,
{
    let world_id = room.world_id;
    // Loaded once per connection (not per event): a per-event read would contend
    // with apply_intent on the single-writer pool. A defaults change mid-session
    // takes effect on the client's next (re)connect.
    let world_defaults = repo.world_cap_defaults(world_id).await.unwrap_or_default();
    // Fail open for the advisory client copy only; server-side enforcement
    // reads requirements freshly per intent and fails closed.
    let world_reqs = welcome_capability_requirements(repo.as_ref(), world_id, &modules_dir).await;
    let world_contracts = match repo.world_contract_declarations(world_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "contract declarations unreadable; sending empty");
            Vec::new()
        }
    };
    // Informational/parity for the client (tier-2 is server-enforced; tier-1
    // validates client-side). Fail open to empty for the advisory copy.
    let world_schemas = match repo.world_schema_declarations(world_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "schema declarations unreadable; sending empty");
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
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            world_default_grants: actor_grants,
            user_role: ctx.world_role,
            capability_requirements: world_reqs,
            contract_declarations: world_contracts,
            schema_declarations: world_schemas,
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
                    if send_filtered(&mut sink, repo.as_ref(), &room, &ctx, &world_defaults, f.as_ref()).await.is_err() { break; }
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
                        // Resolve the effective view context. `as_user` (see-as-player) is
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
                        let (payload, seq, grid, grid_shapes) = {
                            let ecs = room.scene().read().await;
                            (crate::scene::compute_derived(&channel, &ecs, &view_ctx), ecs.committed_seq(), ecs.scene_grid_sizes(), ecs.scene_grid_shapes())
                        };
                        match payload {
                            Some(mut p) => {
                                if channel == "vision" {
                                    enrich_vision_explored(&mut p, &grid, &grid_shapes, repo.as_ref(), world_id, view_ctx.user_id, accumulate).await;
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
                        if send_filtered(&mut sink, repo.as_ref(), &room, &ctx, &world_defaults, msg.as_ref()).await.is_err() { break; }
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
                                // Resolve this connection's active see-as-player target, if any:
                                // a `vision`-channel scene subscription whose resolved `view_ctx`
                                // is a DIFFERENT user than the connection's own (a GM see-as).
                                // Only a GM can hold such a sub — the `SceneSubscribe` handler gates
                                // `as_user` to a GM and server-resolves the target role — so the
                                // `world_role == Gm` guard here is belt-and-suspenders. Vision subs
                                // are world-wide (not scene-scoped); scene-exactness is enforced
                                // inside `clip_move_stream` by computing the target's vision for the
                                // move's own `scene`. The client maintains a single see-as target,
                                // so `find` (first match) is deterministic in practice.
                                let see_as = if ctx.world_role
                                    == crate::data::document::WorldRole::Gm
                                {
                                    scene_subs
                                        .values()
                                        .find(|s| {
                                            s.channel == "vision"
                                                && s.view_ctx.user_id != ctx.user_id
                                        })
                                        .map(|s| s.view_ctx)
                                } else {
                                    None
                                };
                                match clip_move_stream(msg.as_ref(), &ctx, see_as, &room).await {
                                    Some(out) => sink.send(text(&out)).await.is_err(),
                                    None => false, // suppressed: do not send
                                }
                            }
                            ServerMsg::Evicted { user } => {
                                // Targeted eviction. Delivery of the frame is
                                // best-effort; the Close and the `break` are the
                                // point — the ingress loop tears the connection
                                // down when this egress task exits.
                                if user.is_none() || *user == Some(ctx.user_id) {
                                    let _ = sink.send(text(msg.as_ref())).await;
                                    let _ = sink.send(Message::Close(None)).await;
                                    true
                                } else {
                                    false
                                }
                            }
                            other => send_filtered(
                                &mut sink,
                                repo.as_ref(),
                                &room,
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
                let (seq, snapshot, grid, grid_shapes) = {
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
                    (ecs.committed_seq(), out, ecs.scene_grid_sizes(), ecs.scene_grid_shapes())
                };
                for (id, channel, view_ctx, payload) in snapshot {
                    if let Some(mut p) = payload {
                        if channel == "vision" {
                            // See-as (view_ctx != own) is read-only: emit the target's explored, never persist.
                            let accumulate = view_ctx.user_id == ctx.user_id;
                            enrich_vision_explored(&mut p, &grid, &grid_shapes, repo.as_ref(), world_id, view_ctx.user_id, accumulate).await;
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
        send_filtered(sink, repo, room, ctx, world_defaults, f.as_ref()).await?;
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

    // Dual-write fixture helpers (`ws_engine`/`token_engine`) live in `ws::test_support`,
    // shared with `ws::room`'s test module.
    use crate::ws::test_support::{token_engine, ws_engine};

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
            std::path::PathBuf::from("nonexistent-test-modules-dir-for-egress-lag-test"),
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
            room.publish(
                repo.as_ref(),
                &ctx,
                vec![Operation::Create { doc }],
                0,
                WriteOrigin::Client,
            )
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

    #[tokio::test]
    async fn welcome_unions_enabled_modules_requirements_with_gm_authored_ones() {
        use crate::data::document::CapabilityRequirement;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            format!(
                r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/plus","caps":["plus:write"]}}]}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        // A GM-authored requirement, unrelated to any module.
        repo.set_world_cap_requirements(
            world.id,
            &[CapabilityRequirement {
                path_prefix: "/system/vision".into(),
                caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
            }],
        )
        .await
        .unwrap();

        // With nothing enabled, only the GM-authored requirement is published.
        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].path_prefix, "/system/vision");

        // Enabling the module adds its requirement WITHOUT removing the GM's own.
        repo.set_world_enabled_modules(world.id, &["actors-plus".to_string()])
            .await
            .unwrap();
        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().any(|r| r.path_prefix == "/system/vision"));
        assert!(reqs.iter().any(|r| r.path_prefix == "/system/plus"));

        // world_cap_requirements itself is never mutated by this — the raw GM
        // record still holds exactly its one original entry.
        assert_eq!(
            repo.world_cap_requirements(world.id).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn welcome_capability_requirements_unions_caps_for_the_same_path_prefix() {
        use crate::data::document::CapabilityRequirement;
        use std::collections::BTreeSet;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("scene-mod")).unwrap();
        std::fs::write(
            dir.path().join("scene-mod").join("module.json"),
            format!(
                r#"{{"id":"scene-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/scene","caps":["write"]}}]}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        // A GM-authored requirement on "/scene" requiring cap "read", plus a
        // module declaring a requirement on the SAME "/scene" prefix requiring
        // "write".
        repo.set_world_cap_requirements(
            world.id,
            &[CapabilityRequirement {
                path_prefix: "/scene".into(),
                caps: ["read".to_string()].into_iter().collect(),
            }],
        )
        .await
        .unwrap();
        repo.set_world_enabled_modules(world.id, &["scene-mod".to_string()])
            .await
            .unwrap();

        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;

        let scene_reqs: Vec<_> = reqs.iter().filter(|r| r.path_prefix == "/scene").collect();
        assert_eq!(
            scene_reqs.len(),
            1,
            "must not emit two entries for the same path_prefix"
        );
        assert_eq!(
            scene_reqs[0].caps,
            BTreeSet::from(["read".to_string(), "write".to_string()]),
            "caps from both sources must be unioned, not one dropped"
        );
    }

    /// A module that is enabled but whose on-disk manifest
    /// declares an engine range the RUNNING server no longer satisfies (a version
    /// downgrade, or a manifest edited after enable) must NOT publish its
    /// requirements into the advisory Welcome union — mirroring the enable-time
    /// `engine_compat_ok` gate in `module_routes::set_world_enabled_modules`: engine
    /// compatibility is enforced both at enable time and again on every Welcome load,
    /// not just once. Simulated by storing the id directly via `set_world_enabled_modules`
    /// (bypassing the HTTP enable-time gate) against a manifest declaring `^99.0.0`.
    #[tokio::test]
    async fn welcome_excludes_requirements_from_an_enabled_but_now_incompatible_module() {
        use crate::data::document::CapabilityRequirement;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("stale-mod")).unwrap();
        std::fs::write(
            dir.path().join("stale-mod").join("module.json"),
            r#"{"id":"stale-mod","version":"1.0.0","engines":{"shadowcat":"^99.0.0"},"requirements":[{"path_prefix":"/system/stale","caps":["stale:write"]}]}"#,
        )
        .unwrap();

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.set_world_cap_requirements(
            world.id,
            &[CapabilityRequirement {
                path_prefix: "/system/vision".into(),
                caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
            }],
        )
        .await
        .unwrap();

        // Enabled directly at the repo layer (bypassing the HTTP enable-time
        // engine-compat gate), simulating a module that was compatible at enable
        // time but no longer is (server downgrade / manifest edit).
        repo.set_world_enabled_modules(world.id, &["stale-mod".to_string()])
            .await
            .unwrap();

        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert_eq!(
            reqs.len(),
            1,
            "an incompatible enabled module must not contribute requirements"
        );
        assert_eq!(reqs[0].path_prefix, "/system/vision");
        assert!(!reqs.iter().any(|r| r.path_prefix == "/system/stale"));
    }

    /// Behavior-preservation test for the `spawn_blocking` wrap around
    /// `scan_installed_modules` in `welcome_capability_requirements`: proves the
    /// Welcome path still resolves an enabled module's declared requirements
    /// correctly when the scan runs off the async worker thread. Not a
    /// red/green blocking-detection test (blocking-vs-non-blocking isn't
    /// directly unit-testable) — mirrors
    /// `welcome_unions_enabled_modules_requirements_with_gm_authored_ones`'s
    /// setup, verifying the `spawn_blocking`-wrapped path yields results
    /// identical to the direct (non-blocking) path.
    #[tokio::test]
    async fn welcome_capability_requirements_still_resolves_module_requirements_via_spawn_blocking()
    {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("blocking-mod")).unwrap();
        std::fs::write(
            dir.path().join("blocking-mod").join("module.json"),
            format!(
                r#"{{"id":"blocking-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/blocking","caps":["blocking:write"]}}]}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.set_world_enabled_modules(world.id, &["blocking-mod".to_string()])
            .await
            .unwrap();

        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert!(
            reqs.iter().any(|r| r.path_prefix == "/system/blocking"),
            "module-declared requirements must still resolve correctly when scan runs via spawn_blocking"
        );
    }

    /// Build the square `GridShape` companion map the production `enrich_vision_explored` captures
    /// via `SceneEcs::scene_grid_shapes` — one `SquareGrid` per scene at its cell size, so a
    /// square-grid test indexes explored fog byte-identically to the pre-migration hardcoded math.
    fn square_grid_shapes(
        grid: &std::collections::HashMap<Uuid, f64>,
    ) -> std::collections::HashMap<Uuid, Box<dyn crate::scene::grid_shape::GridShape + Send + Sync>>
    {
        grid.iter()
            .map(|(&scene, &cell)| {
                (
                    scene,
                    Box::new(crate::scene::grid_shape::SquareGrid {
                        cell,
                        rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
                    })
                        as Box<dyn crate::scene::grid_shape::GridShape + Send + Sync>,
                )
            })
            .collect()
    }

    /// The dispatch-layer accumulation: a masked vision payload grows + persists the player's
    /// explored fog and gains a scene-tagged `explored` set; a revisit re-emits without growing; a
    /// GM `mode:"all"` payload is untouched (no fog → no explored).
    #[tokio::test]
    async fn enrich_accumulates_persists_and_emits_explored() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = Uuid::from_u128(1);
        let scene = Uuid::from_u128(10);
        let user = Uuid::from_u128(20);
        let grid = std::collections::HashMap::from([(scene, 100.0)]);
        let grid_shapes = square_grid_shapes(&grid);

        // A masked payload with a visibility polygon covering a 3×3 cell block in `scene`.
        let mut payload = json!({
            "mode": "masked",
            "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
        });
        enrich_vision_explored(&mut payload, &grid, &grid_shapes, &repo, world, user, true).await;

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
        enrich_vision_explored(&mut again, &grid, &grid_shapes, &repo, world, user, true).await;
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
        enrich_vision_explored(&mut gm, &grid, &grid_shapes, &repo, world, user, true).await;
        assert_eq!(gm, json!({ "mode": "all" }));
    }

    /// A scene absent from BOTH grid maps has no live scene document — `enrich_vision_explored`
    /// must skip it (fail closed), never synthesize a fallback square grid to index against.
    #[tokio::test]
    async fn enrich_skips_scene_absent_from_grid_maps() {
        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let world = Uuid::from_u128(1);
        let user = Uuid::from_u128(2);
        let ghost = Uuid::from_u128(0xDEAD);
        let grid: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
        let shapes = square_grid_shapes(&grid);

        let mut payload = json!({
            "mode": "masked",
            "polygons": [{ "scene": ghost, "points": [0.0, 0.0, 200.0, 0.0, 200.0, 200.0] }]
        });
        enrich_vision_explored(
            &mut payload,
            &grid,
            &shapes,
            repo.as_ref(),
            world,
            user,
            true,
        )
        .await;

        let explored = payload.get("explored").and_then(|e| e.as_array());
        assert!(
            explored.map(|a| a.is_empty()).unwrap_or(true),
            "no explored entry for a scene with no grid entry"
        );
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
        let grid_shapes = square_grid_shapes(&grid);

        // Seed the target with one explored cell (as if they'd been there).
        let mut seed = crate::scene::explored::ExploredSet::new();
        seed.mark_polygons(
            &[vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0]],
            &crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
            },
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
        enrich_vision_explored(
            &mut payload,
            &grid,
            &grid_shapes,
            &repo,
            world,
            target,
            false,
        )
        .await;
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
            WriteOrigin::Client,
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
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Player-owned token at (50,50) = cell (0,0). The player sees nothing (dark scene).
        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
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
            None,
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
            None,
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

    /// `scene_ping_permitted` admits a token-less reader and refuses a foreign-world scene, a
    /// scene whose `permissions.default` denies READ, and a ghost id.
    #[tokio::test]
    async fn scene_ping_guard_admits_reader_refuses_foreign_and_hidden() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
        use crate::ws::room::RoomRegistry;

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        let other_world = repo.create_world_owned("X", gm, 0).await.unwrap();
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, p, WorldRole::Spectator)
            .await
            .unwrap();
        let spectator = PermissionContext {
            user_id: p,
            world_role: WorldRole::Spectator,
        };
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let gm_ctx_other = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };

        let reg = RoomRegistry::new();
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let room_other = reg
            .get_or_create(repo.as_ref(), other_world.id)
            .await
            .unwrap()
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;

        let (vis_id, foreign_id, hidden_id) = (
            Uuid::from_u128(0xB001),
            Uuid::from_u128(0xB002),
            Uuid::from_u128(0xB003),
        );

        // A readable scene in this world (default DocRole is `None`, so an explicit
        // `Observer` default is required for the spectator to hold READ).
        let mut vis = wdoc(world.id, vis_id, "scene");
        vis.owner = Some(gm);
        vis.permissions.default = DocRole::Observer;
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: vis }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // A scene in a DIFFERENT world (same GM authoring it).
        let mut foreign = wdoc(other_world.id, foreign_id, "scene");
        foreign.owner = Some(gm);
        room_other
            .publish(
                repo.as_ref(),
                &gm_ctx_other,
                vec![crate::data::command::Operation::Create { doc: foreign }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        // A scene in this world whose default grants nobody READ.
        let mut hidden = wdoc(world.id, hidden_id, "scene");
        hidden.owner = Some(gm);
        hidden.permissions.default = DocRole::None;
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: hidden }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Token-less spectator may ping the scene they can read...
        assert!(scene_ping_permitted(vis_id, &spectator, world.id, repo.as_ref()).await);
        // ...but not a scene in another world, a scene that denies READ, or a ghost id.
        assert!(!scene_ping_permitted(foreign_id, &spectator, world.id, repo.as_ref()).await);
        assert!(!scene_ping_permitted(hidden_id, &spectator, world.id, repo.as_ref()).await);
        assert!(
            !scene_ping_permitted(Uuid::from_u128(0xDEAD), &spectator, world.id, repo.as_ref())
                .await
        );
    }

    /// A `Pathfind` naming a scene the requester controls no token in is refused, even when that
    /// scene is `unrestricted` (no visibility mask to fail closed on). Otherwise a player could
    /// route-preview inside a scene they have never entered and read its `blocksMove` wall layout
    /// off the returned polyline. The GM is unaffected, and the requester's own scene still routes.
    #[tokio::test]
    async fn pathfind_refuses_a_scene_the_requester_controls_no_token_in() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
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
        let (scene_a, scene_b, token_id, wall_id, ws_id) = (
            Uuid::from_u128(0xB001),
            Uuid::from_u128(0xB002),
            Uuid::from_u128(0xB003),
            Uuid::from_u128(0xB004),
            Uuid::from_u128(0xB005),
        );

        // Unrestricted world-wide: no visibility mask anywhere, so nothing else fails closed.
        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        for id in [scene_a, scene_b] {
            let mut scene = wdoc(world.id, id, "scene");
            scene.owner = Some(author);
            scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![crate::data::command::Operation::Create { doc: scene }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        // The player's only token lives in A.
        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_a);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // B holds wall geometry the player must not be able to probe.
        let mut wall = wdoc(world.id, wall_id, "wall");
        wall.parent_id = Some(scene_b);
        wall.owner = Some(author);
        wall.engine = Some(
            json!({ "seg": { "x1": 200, "y1": -100, "x2": 200, "y2": 100 }, "blocksMove": true }),
        );
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let rid = Uuid::from_u128(0xF002);

        let leaked = handle_pathfind(
            rid,
            scene_b,
            (50.0, 50.0),
            vec![(450.0, 50.0)],
            0.1,
            None,
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(leaked, ServerMsg::PathError { .. }),
            "a player with no token in B must not route inside B; got {leaked:?}"
        );

        // The same cross-scene probe, now naming a token the requester DOES own in scene A. The
        // presence gate must still refuse for scene B: naming a token is not presence in a scene.
        let leaked_with_named_token = handle_pathfind(
            rid,
            scene_b,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.4,
            Some(token_id),
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(leaked_with_named_token, ServerMsg::PathError { .. }),
            "naming an owned token grants no presence in another scene; got {leaked_with_named_token:?}"
        );

        // Control: the player's own scene still routes (the guard cannot break play).
        let own = handle_pathfind(
            rid,
            scene_a,
            (50.0, 50.0),
            vec![(450.0, 50.0)],
            0.1,
            None,
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(own, ServerMsg::PathResult { .. }),
            "the requester's own scene must still route; got {own:?}"
        );

        // The GM routes in any scene — presence is a non-GM gate only.
        let gm = handle_pathfind(
            rid,
            scene_b,
            (50.0, 50.0),
            vec![(450.0, 50.0)],
            0.1,
            None,
            &gm_ctx,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(gm, ServerMsg::PathResult { .. }),
            "GM routing is unaffected; got {gm:?}"
        );
    }

    /// `Pathfind` naming a token the requester does not effectively own is refused generically —
    /// the named token is a footprint SOURCE, not a delegated presence grant, so an attacker
    /// cannot read a stranger's token size (or probe gaps sized to it) by naming an id they do
    /// not control. Both requesters have their OWN token in the scene, so the Step-0 presence
    /// gate passes for both — isolating this test to the token-ownership check (Step 4).
    #[tokio::test]
    async fn pathfind_naming_an_unowned_token_is_refused() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
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
        let pa = repo
            .create_user("player-a", None, ServerRole::User, 0)
            .await
            .unwrap();
        let pb = repo
            .create_user("player-b", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, pa, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(world.id, pb, WorldRole::Player)
            .await
            .unwrap();
        let player_a = PermissionContext {
            user_id: pa,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg
            .get_or_create(repo.as_ref(), world.id)
            .await
            .unwrap()
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, ws_id, token_a, token_b) = (
            Uuid::from_u128(0xC001),
            Uuid::from_u128(0xC002),
            Uuid::from_u128(0xC003),
            Uuid::from_u128(0xC004),
        );

        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut ta = wdoc(world.id, token_a, "token");
        ta.parent_id = Some(scene_id);
        ta.owner = Some(pa);
        ta.permissions.users.insert(pa, DocRole::Owner);
        ta.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ta }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Player B's own token — the one player A will name without owning it.
        let mut tb = wdoc(world.id, token_b, "token");
        tb.parent_id = Some(scene_id);
        tb.owner = Some(pb);
        tb.permissions.users.insert(pb, DocRole::Owner);
        tb.engine = Some(token_engine(60.0, 60.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: tb }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let rid = Uuid::from_u128(0xF010);
        let res = handle_pathfind(
            rid,
            scene_id,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.4,
            Some(token_b),
            &player_a,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(res, ServerMsg::PathError { .. }),
            "an unowned token is refused generically; got {res:?}"
        );
    }

    /// Guards the cross-scene axis: the requester has their OWN token in the named scene (so
    /// Step-0 presence passes), but names a token they own in a DIFFERENT scene — the footprint
    /// derivation (Step 4) must still refuse, isolating this from the presence gate.
    #[tokio::test]
    async fn pathfind_naming_a_token_in_another_scene_is_refused() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
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
        let (scene_a, scene_b, ws_id, token_in_a, token_in_b) = (
            Uuid::from_u128(0xC101),
            Uuid::from_u128(0xC102),
            Uuid::from_u128(0xC103),
            Uuid::from_u128(0xC104),
            Uuid::from_u128(0xC105),
        );

        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        for id in [scene_a, scene_b] {
            let mut scene = wdoc(world.id, id, "scene");
            scene.owner = Some(author);
            scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![crate::data::command::Operation::Create { doc: scene }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        // The token the attacker will name while requesting B — owned by the player, but
        // parented to A.
        let mut ta = wdoc(world.id, token_in_a, "token");
        ta.parent_id = Some(scene_a);
        ta.owner = Some(p);
        ta.permissions.users.insert(p, DocRole::Owner);
        ta.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ta }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // The player's OWN token in B, so the Step-0 presence gate for B passes independently of
        // the named token — isolating this test to Step 4's cross-scene check.
        let mut tb = wdoc(world.id, token_in_b, "token");
        tb.parent_id = Some(scene_b);
        tb.owner = Some(p);
        tb.permissions.users.insert(p, DocRole::Owner);
        tb.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: tb }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let rid = Uuid::from_u128(0xF011);
        let res = handle_pathfind(
            rid,
            scene_b,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.4,
            Some(token_in_a),
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(res, ServerMsg::PathError { .. }),
            "a token outside the named scene is refused; got {res:?}"
        );
    }

    /// Builds a scene with a 1-cell-wide corridor (walls at x=100 and x=200, cell=100) and one
    /// owned, actor-linked token whose real footprint (radius 1.0 cell, `circle` shape, size
    /// 2.0x2.0) is far too wide for the 100-unit corridor (which admits at most radius 0.5 cell).
    /// Shared by the "ignores a lying wire footprint" (Step 4 derives and refuses) and "no token
    /// honors the wire value" (Step 4 is skipped entirely) tests below.
    async fn harness_with_narrow_corridor_and_large_owned_token() -> (
        Arc<SqliteRepository>,
        Arc<crate::ws::room::Room>,
        crate::data::membership::PermissionContext,
        Uuid,
        Uuid,
    ) {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
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
        let (scene_id, ws_id, wall_1, wall_2, actor_id, token_id) = (
            Uuid::from_u128(0xC201),
            Uuid::from_u128(0xC202),
            Uuid::from_u128(0xC203),
            Uuid::from_u128(0xC204),
            Uuid::from_u128(0xC205),
            Uuid::from_u128(0xC206),
        );

        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Two parallel walls one cell apart (x=100, x=200), forming a north-south corridor a
        // vertical route can traverse (at x=150) without crossing either wall directly.
        for (id, x) in [(wall_1, 100.0), (wall_2, 200.0)] {
            let mut wall = wdoc(world.id, id, "wall");
            wall.parent_id = Some(scene_id);
            wall.owner = Some(author);
            wall.engine = Some(
                json!({ "seg": { "x1": x, "y1": -400.0, "x2": x, "y2": 400.0 }, "blocksMove": true }),
            );
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![crate::data::command::Operation::Create { doc: wall }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        // The linked actor supplies the token's REAL size: a 2.0x2.0 circle ⇒ radius 1.0 cell —
        // far wider than the 100-unit (1-cell) corridor's 0.5-cell admissible radius.
        let mut actor = wdoc(world.id, actor_id, "actor");
        actor.owner = Some(author);
        actor.engine = Some(json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 2.0, "h": 2.0 },
            "shape": "circle",
            "conditions": [],
            "prototype": true,
        }));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(json!({
            "x": 150.0, "y": 350.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
            "actor_id": actor_id.to_string(),
        }));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        (repo, room, player, scene_id, token_id)
    }

    /// The wire value lies (0.01, tiny) but the named token's real, derived footprint (radius 1.0
    /// cell) does not fit the 1-cell corridor: the derived value must win, refusing the route.
    #[tokio::test]
    async fn pathfind_naming_an_owned_token_ignores_a_lying_wire_footprint() {
        let (repo, room, player, scene_id, token_id) =
            harness_with_narrow_corridor_and_large_owned_token().await;
        let rid = Uuid::from_u128(0xF012);
        let res = handle_pathfind(
            rid,
            scene_id,
            (150.0, 350.0),
            vec![(150.0, -350.0)],
            0.01,
            Some(token_id),
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(res, ServerMsg::PathError { .. }),
            "the derived footprint does not fit the corridor, so no route is returned; got {res:?}"
        );
    }

    /// The SAME corridor and token, but no token is named: the wire's tiny 0.01 radius is honored
    /// verbatim and fits, so the token-less preview routes through.
    #[tokio::test]
    async fn pathfind_without_a_token_uses_the_wire_footprint() {
        let (repo, room, player, scene_id, _token_id) =
            harness_with_narrow_corridor_and_large_owned_token().await;
        let rid = Uuid::from_u128(0xF013);
        let res = handle_pathfind(
            rid,
            scene_id,
            (150.0, 350.0),
            vec![(150.0, -350.0)],
            0.01,
            None,
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(
            matches!(res, ServerMsg::PathResult { .. }),
            "a token-less preview honors the wire radius; got {res:?}"
        );
    }

    /// `resolve_token_footprint` returns `None` for a derived radius over `MAX_FOOTPRINT_CELLS` —
    /// the handler must refuse, never fall back to the wire value (which would reopen the
    /// understated-footprint hole a derived footprint exists to close).
    #[tokio::test]
    async fn pathfind_refuses_an_oversized_derived_footprint() {
        use crate::auth::role::ServerRole;
        use crate::data::document::{DocRole, WorldRole};
        use crate::data::membership::PermissionContext;
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
        let (scene_id, ws_id, actor_id, token_id) = (
            Uuid::from_u128(0xC301),
            Uuid::from_u128(0xC302),
            Uuid::from_u128(0xC303),
            Uuid::from_u128(0xC304),
        );

        let mut ws = wdoc(world.id, ws_id, "world-settings");
        ws.owner = Some(author);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world.id, scene_id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // w=h=1000 ⇒ ~707 cells, far over MAX_FOOTPRINT_CELLS (64.0).
        let mut actor = wdoc(world.id, actor_id, "actor");
        actor.owner = Some(author);
        actor.engine = Some(json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1000.0, "h": 1000.0 },
            "shape": "square",
            "conditions": [],
            "prototype": true,
        }));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(json!({
            "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
            "actor_id": actor_id.to_string(),
        }));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let rid = Uuid::from_u128(0xF014);
        let res = handle_pathfind(
            rid,
            scene_id,
            (50.0, 50.0),
            vec![(250.0, 50.0)],
            0.4,
            Some(token_id),
            &player,
            &room,
            repo.as_ref(),
        )
        .await;
        assert!(matches!(res, ServerMsg::PathError { .. }), "got {res:?}");
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
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
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
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Player-owned token at (50,50). Start = cell-center (0,0); goal = (150,50) = one step right.
        let mut token = wdoc(world.id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
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
                    mover_vision.is_some(),
                    "a non-GM mover must get a progressive vision sweep, even in an \
                     Unrestricted-mode scene (gated on role, not restriction mode)"
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
        let grid_shapes = square_grid_shapes(&grid);
        let mut payload = json!({ "mode": "masked", "polygons": [] });
        enrich_vision_explored(
            &mut payload,
            &grid,
            &grid_shapes,
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
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        if let Some(pos) = obs_token_pos {
            let token_id = Uuid::from_u128(0xE002);
            let mut tok = wdoc(world.id, token_id, "token");
            tok.parent_id = Some(scene_id);
            tok.owner = Some(obs);
            tok.permissions.users.insert(obs, DocRole::Owner);
            tok.engine = Some(token_engine(pos.0, pos.1));
            room.publish(
                repo.as_ref(),
                &gm_ctx,
                vec![Operation::Create { doc: tok }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        if let Some(ws) = wall_system {
            let wall_id = Uuid::from_u128(0xE003);
            let mut wall = wdoc(world.id, wall_id, "wall");
            wall.parent_id = Some(scene_id);
            wall.owner = Some(gm);
            wall.engine = Some(ws);
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
                WriteOrigin::Client,
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
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &ctx, None, &room).await;

        assert!(result.is_some(), "mover must receive a frame");
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv_out,
                cost,
                ..
            } => {
                assert_eq!(s, samples, "mover receives all samples unchanged");
                assert_eq!(mv_out, mv, "mover receives mover_vision unchanged");
                assert_eq!(cost, Some(2.0), "mover receives the true cost unchanged");
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
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

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
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

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
                cost,
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
                // Critical security fix regression: a clipped observer must never learn the
                // true authoritative cost — it may reflect secret (gm_only) region terrain
                // the observer's clipped samples never reveal.
                assert_eq!(
                    cost, None,
                    "cost must be nulled for a clipped observer (secrecy: must not disclose \
                     authoritative cost, which may include secret-region terrain)"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// Same near-side/occluded clip boundary as `clip_observer_sees_near_side_prefix`, but over
    /// a genuinely any-angle (non-axis-aligned) path — proves the per-recipient egress clip
    /// is engine-agnostic geometry, unaffected by whether the sampled polyline is grid-stepped or
    /// continuous. Wall at x=100 (unchanged); observer at (50,50) sees anything with
    /// x<100 regardless of y, so the diagonal y-offsets below don't change the visibility split.
    #[tokio::test]
    async fn clip_observer_sees_near_side_prefix_any_angle_diagonal_path() {
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
            duration_ms: 1500.0,
            stop: [310.0, 10.0],
            samples: vec![
                PosSample {
                    t_ms: 0.0,
                    pos: [50.0, 60.0], // near side, diagonal offset — visible
                },
                PosSample {
                    t_ms: 750.0,
                    pos: [140.0, 95.0], // behind wall, diagonal — occluded
                },
                PosSample {
                    t_ms: 1500.0,
                    pos: [310.0, 10.0], // further behind wall, diagonal — occluded
                },
            ],
            mover_vision: None,
            cost: Some(3.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

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
                cost,
                truncated,
                ..
            } => {
                assert_eq!(
                    s.len(),
                    1,
                    "only the near-side diagonal sample is visible; got {} samples: {s:?}",
                    s.len()
                );
                assert_eq!(s[0].pos, [50.0_f64, 60.0_f64]);
                assert_eq!(mv, None, "mover_vision must be None for observers");
                assert_eq!(
                    out_stop,
                    [50.0_f64, 60.0_f64],
                    "stop clips to the last visible sample, not the true diagonal goal"
                );
                // Critical 2 regression: duration_ms must be clipped to the last visible
                // sample's t_ms, NOT the true goal/full travel duration (mirrors the
                // axis-aligned sibling `clip_observer_sees_near_side_prefix`).
                assert!(
                    (out_duration_ms - 0.0_f64).abs() < 1e-9,
                    "duration_ms must be clipped to last visible sample t_ms (0 ms), got {out_duration_ms}"
                );
                assert_eq!(cost, None, "cost must be nulled for a clipped observer");
                assert_eq!(
                    truncated, None,
                    "truncated must be nulled for a clipped observer"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// A `gm_only` (`DocRole::None`) `blocksSight` wall bounds the observer's authoritative
    /// vision identically to a normal wall — `sight_walls` is permission-blind, returning
    /// every wall regardless of visibility tier. When the mover's entire path lies behind
    /// the secret wall, the frame is
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
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

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
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &gm_ctx, None, &room).await;

        assert!(result.is_some(), "GM must receive a frame");
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv_out,
                stop: out_stop,
                duration_ms: out_duration_ms,
                cost,
                ..
            } => {
                assert_eq!(s, samples, "GM receives all samples unchanged");
                assert_eq!(mv_out, None, "mover_vision must be nulled for GM observers");
                assert_eq!(out_stop, true_stop, "GM receives the full stop (no clip)");
                assert!(
                    (out_duration_ms - true_duration_ms).abs() < 1e-9,
                    "GM receives the full duration_ms (no clip)"
                );
                assert_eq!(
                    cost,
                    Some(2.0),
                    "GM receives the true cost unchanged (trusted, full information)"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// A GM previewing AS a specific player (`see_as = Some(target)`) whose token is in the
    /// move's scene has their OWN view narrowed to what that player would actually see mid-move,
    /// via the same clip path a real observer gets. Setup mirrors
    /// `clip_observer_sees_near_side_prefix`: target token at (50,50), vertical `blocksSight`
    /// wall at x=100, samples at (50,50)/(150,50)/(250,50). Only the near-side sample is visible.
    ///
    /// This is the core behavior change — before threading `see_as`, the GM branch ALWAYS
    /// returned the full unclipped stream regardless of the active see-as target.
    #[tokio::test]
    async fn clip_gm_see_as_clips_to_target_vision() {
        use crate::ws::protocol::PosSample;

        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        // `obs` is the see-as target: a player with a token at (50,50).
        let (room, gm_ctx, target_ctx, scene_id) =
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
                    pos: [50.0, 50.0], // near side — target can see this
                },
                PosSample {
                    t_ms: 200.0,
                    pos: [150.0, 50.0], // behind wall — occluded from the target
                },
                PosSample {
                    t_ms: 400.0,
                    pos: [250.0, 50.0], // further behind — occluded from the target
                },
            ],
            mover_vision: None,
            cost: Some(2.0),
            truncated: Some(false),
        };

        // GM previewing as `target`.
        let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

        assert!(
            result.is_some(),
            "GM see-as preview with a visible sample must receive a clipped frame"
        );
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv,
                stop: out_stop,
                duration_ms: out_duration_ms,
                cost,
                truncated,
                ..
            } => {
                assert_eq!(
                    s.len(),
                    1,
                    "GM see-as must be clipped to the target's vision (near-side only); got {} \
                     samples: {s:?}",
                    s.len()
                );
                assert_eq!(s[0].pos, [50.0_f64, 50.0_f64]);
                assert_eq!(mv, None, "mover_vision stays None for a see-as preview");
                assert_eq!(
                    out_stop,
                    [50.0_f64, 50.0_f64],
                    "stop clips to the target's last visible sample, not the true goal"
                );
                assert!(
                    (out_duration_ms - 0.0_f64).abs() < 1e-9,
                    "duration_ms clips to the target's last visible sample t_ms, got {out_duration_ms}"
                );
                assert_eq!(
                    cost, None,
                    "cost nulled for a clipped see-as preview (same secrecy as a real observer)"
                );
                assert_eq!(
                    truncated, None,
                    "truncated nulled for a clipped see-as preview (same secrecy as a real observer)"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// A see-as whose target has NO vision source in the move's scene (their token is in a
    /// DIFFERENT scene) must NOT clip — the see-as is not applicable to this scene, so the GM
    /// keeps the full unclipped stream (today's plain-GM behavior). Scene-exactness guard: the
    /// target's token lives in `scene_id`, but the move targets an unrelated scene, so
    /// `player_vision_polygons(target)` (tagged with `scene_id`) filters to empty for the move's
    /// scene.
    #[tokio::test]
    async fn clip_gm_see_as_different_scene_not_clipped() {
        use crate::ws::protocol::PosSample;

        // Target token at (50,50) in `scene_id`; a wall that WOULD occlude if it applied.
        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, gm_ctx, target_ctx, _scene_id) =
            setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

        // The move happens in a DIFFERENT scene where the target has no token.
        let other_scene = Uuid::from_u128(0xE099);
        let mover_id = Uuid::from_u128(0xAABB);
        let samples = vec![
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
        ];
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: other_scene,
            start_server_ms: 1000.0,
            duration_ms: 600.0,
            stop: [250.0, 50.0],
            samples: samples.clone(),
            mover_vision: None,
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

        assert!(
            result.is_some(),
            "different-scene see-as must not suppress the GM's frame"
        );
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s, cost, ..
            } => {
                assert_eq!(
                    s, samples,
                    "a see-as for a different scene must not clip — GM keeps all samples"
                );
                assert_eq!(
                    cost,
                    Some(2.0),
                    "a see-as for a different scene must not null the GM's cost"
                );
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }

    /// A see-as target who IS in the move's scene but cannot see ANY sample (the whole move is
    /// behind a `gm_only` wall, from the target's viewpoint) is suppressed entirely — the GM's
    /// faithful preview shows zero frames, exactly like the previewed player would receive.
    #[tokio::test]
    async fn clip_gm_see_as_fully_occluded_suppressed() {
        use crate::ws::protocol::PosSample;

        // gm_only wall at x=100; target token at (50,50) cannot see x>100.
        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, gm_ctx, target_ctx, scene_id) =
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
                    pos: [150.0, 50.0], // behind the wall from the target's viewpoint
                },
                PosSample {
                    t_ms: 200.0,
                    pos: [250.0, 50.0], // further behind
                },
            ],
            mover_vision: None,
            cost: Some(2.0),
            truncated: Some(false),
        };

        let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

        assert!(
            result.is_none(),
            "a see-as preview of a wholly-occluded move must be suppressed (None); got {result:?}"
        );
    }
}
