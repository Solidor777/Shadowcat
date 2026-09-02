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
use crate::ws::MESSAGE_RATE_PER_MIN;

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

/// Redact a `StoredCommand`-carrying `Event` frame for `ctx` (per-recipient, seq-preserving)
/// and send it, reduced to the plain wire `ServerMsg::Event` at this — the ONLY — point where it
/// is serialized. Used for live broadcast delivery AND replay (the same path).
async fn send_filtered_event<S>(
    sink: &mut S,
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    stored: &crate::data::snapshot::StoredCommand,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    // Loads complete BEFORE the guard: no lock across await. The guard is held only around the
    // synchronous core — the same short-read-guard discipline as clip_move_stream.
    let current = crate::data::permission::load_current_docs(repo, &stored.command).await;
    let filtered = {
        let ecs = room.scene().read().await;
        crate::data::permission::filter_command(
            &stored.command,
            &stored.snapshot,
            ctx,
            world_defaults,
            &current,
            |id| ecs.actor(id),
        )
    };
    let out = ServerMsg::Event {
        command: filtered,
        intent_id: None,
    };
    sink.send(text(&out)).await.map_err(|_| ())
}

/// Send a non-`Event` broadcast frame unchanged. `MoveStream` must never reach here — it
/// requires per-recipient clipping in the egress loop (`clip_move_stream`); this guard catches a
/// future routing regression at test time.
async fn send_plain<S>(sink: &mut S, msg: &ServerMsg) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    debug_assert!(
        !matches!(msg, ServerMsg::MoveStream { .. }),
        "MoveStream must be clipped per-recipient in egress_loop, not sent via send_plain"
    );
    sink.send(text(msg)).await.map_err(|_| ())
}

/// Dispatch a `RoomEvent` to its wire representation: `Event` frames are redacted per-recipient
/// via `send_filtered_event`; every other frame passes through `send_plain` unchanged. Shared by
/// live broadcast delivery and replay (`replay`).
async fn send_room_event<S>(
    sink: &mut S,
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    ev: &crate::ws::room::RoomEvent,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    match ev {
        crate::ws::room::RoomEvent::Event(stored) => {
            send_filtered_event(sink, repo, room, ctx, world_defaults, stored).await
        }
        crate::ws::room::RoomEvent::Other(msg) => send_plain(sink, msg.as_ref()).await,
    }
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

    // Lazy world-config reseed: backfills a pre-existing world and self-heals
    // a deleted singleton on the next join; a failed pass (a lost seed race
    // is swallowed inside) degrades to a log line — a join never fails here.
    if let Err(e) = reseed_world_config(&room, repo.as_ref(), &state.config.modules_path()).await {
        tracing::warn!(world = %world_id, error = %e, "world-config reseed failed; continuing join");
    }

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
    let module_scan_cache = state.ws.module_scan_cache.clone();
    let mut egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        EgressConnState {
            room: egress_room,
            repo: egress_repo,
            ctx,
            current_seq,
            modules_dir,
            module_scan_cache,
        },
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
                                    // Every WS-originated write always publishes as
                                    // `WriteOrigin::Client` below — `ClientMsg::Intent`
                                    // carries no field a client could set to select
                                    // `WriteOrigin::ServerMessageRevision` or
                                    // `WriteOrigin::CombatTransition`; both are
                                    // constructed only by trusted internal callers.
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
                                Ok(ClientMsg::Hello { last_seq, .. }) => {
                                    // `world` is redundant here: the connection's own
                                    // `world_id`/`room` were already resolved from the WS
                                    // upgrade route/auth, not from this frame — `Hello` only
                                    // signals cold-start-vs-reconnect in-band. `None` establishes
                                    // this user's resync floor at the room's current seq;
                                    // `Some(_)` (a reconnect reporting real progress) does not
                                    // touch it.
                                    if last_seq.is_none() {
                                        room.establish_resync_floor(ctx.user_id).await;
                                    }
                                }
                                Ok(ClientMsg::Pong) => {}
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
                                    // `message` is the classified, no-leak `Display` text. A non-empty
                                    // pending-enrichment list is run AFTER publish returns, off the
                                    // request path (`chat::post_publish::run_pending_enrichments`).
                                    match crate::chat::handle_send_message(
            crate::chat::MessageRequestCtx {
                room: &room,
                repo: repo.as_ref(),
                ctx: &ctx,
                rate: &message_rate,
                preview: crate::chat::LinkPreviewDeps { client: &preview_client, cache: &preview_cache, rate: &preview_rate },
                now: now_millis(),
                budget_per_min: MESSAGE_RATE_PER_MIN,
            },
            channel,
            content,
            actor_owner,
            audience,
        )
                                    .await
                                    {
                                        Ok((cmd, pending)) => {
                                            if !pending.is_empty() {
                                                if let Some(message_id) = crate::chat::command_message_id(&cmd) {
                                                    tokio::spawn(crate::chat::run_pending_enrichments(
                                                        crate::chat::PostPublishDeps {
                                                            room: room.clone(),
                                                            repo: repo.clone(),
                                                            client: preview_client.clone(),
                                                            assets_root: state.config.assets_path(),
                                                            retain_originals: state.config.retain_originals,
                                                            write_barrier: state.write_barrier.clone(),
                                                            preview_fetch_locks: state.preview_fetch_locks.clone(),
                                                        },
                                                        message_id,
                                                        world_id,
                                                        pending,
                                                    ));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(world = %world_id, user = %user_id, ?e, "message rejected");
                                            if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                                request_id,
                                                message: e.to_string(),
                                            }))).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Ok(ClientMsg::EditMessage { request_id, message_id: edit_message_id, content }) => {
                                    // Same confirm-by-broadcast-echo shape as SendMessage; a rejection is
                                    // surfaced to the sender only via a `request_id`-correlated `ChatError`.
                                    // A non-empty pending-enrichment list is run AFTER publish returns,
                                    // off the request path, same as SendMessage.
                                    match crate::chat::handle_edit_message(
            crate::chat::MessageRequestCtx {
                room: &room,
                repo: repo.as_ref(),
                ctx: &ctx,
                rate: &message_rate,
                preview: crate::chat::LinkPreviewDeps { client: &preview_client, cache: &preview_cache, rate: &preview_rate },
                now: now_millis(),
                budget_per_min: MESSAGE_RATE_PER_MIN,
            },
            edit_message_id,
            content,
        )
                                    .await
                                    {
                                        Ok((cmd, pending)) => {
                                            if !pending.is_empty() {
                                                if let Some(message_id) = crate::chat::command_message_id(&cmd) {
                                                    tokio::spawn(crate::chat::run_pending_enrichments(
                                                        crate::chat::PostPublishDeps {
                                                            room: room.clone(),
                                                            repo: repo.clone(),
                                                            client: preview_client.clone(),
                                                            assets_root: state.config.assets_path(),
                                                            retain_originals: state.config.retain_originals,
                                                            write_barrier: state.write_barrier.clone(),
                                                            preview_fetch_locks: state.preview_fetch_locks.clone(),
                                                        },
                                                        message_id,
                                                        world_id,
                                                        pending,
                                                    ));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(world = %world_id, user = %user_id, ?e, "edit rejected");
                                            if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                                request_id,
                                                message: e.to_string(),
                                            }))).await.is_err() {
                                                break;
                                            }
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
                                Ok(ClientMsg::RecalcRoll {
                                    request_id,
                                    message_id,
                                    roll_id,
                                    ops,
                                }) => {
                                    // Same confirm-by-broadcast-echo shape as
                                    // SendMessage/EditMessage/DeleteMessage; a
                                    // rejection is surfaced to the sender only via a
                                    // correlated `ChatError`.
                                    if let Err(e) = crate::chat::handle_recalc_roll(
                                        crate::chat::RecalcRollRequestCtx {
                                            room: &room,
                                            repo: repo.as_ref(),
                                            ctx: &ctx,
                                            rate: &message_rate,
                                            now: now_millis(),
                                            budget_per_min: MESSAGE_RATE_PER_MIN,
                                        },
                                        message_id,
                                        roll_id,
                                        ops.into_iter()
                                            .map(crate::chat::WireRecalcOp::into_recalc_op)
                                            .collect(),
                                    )
                                    .await
                                    {
                                        tracing::debug!(world = %world_id, user = %user_id, ?e, "recalc rejected");
                                        if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                            request_id,
                                            message: e.to_string(),
                                        }))).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Ok(
                                    m @ ClientMsg::CombatStart { .. }
                                    | m @ ClientMsg::CombatPause { .. }
                                    | m @ ClientMsg::CombatEnd { .. }
                                    | m @ ClientMsg::CombatAdvance { .. }
                                    | m @ ClientMsg::CombatRewind { .. }
                                    | m @ ClientMsg::CombatSort { .. }
                                    | m @ ClientMsg::CombatRoll { .. }
                                    | m @ ClientMsg::CombatResource { .. },
                                ) => {
                                    // Dispatch, authz, dice-context resolution and the one-command
                                    // commit all live in `combat::handle_combat_intent`. `None` on
                                    // success (the broadcast `Event` is the notification, the same
                                    // asymmetric reply protocol as SendMessage); a rejection is
                                    // surfaced to the sender only via a correlated `CombatError`.
                                    // Reuses `message_rate` (the same per-user flood budget every
                                    // other server-authored write handler shares) — checked inside
                                    // `handle_combat_intent` itself, before its snapshot doc read.
                                    if let Some(f) = crate::combat::handle_combat_intent(
                                        &room,
                                        repo.as_ref(),
                                        &ctx,
                                        m,
                                        now_millis(),
                                        &message_rate,
                                    )
                                    .await
                                    {
                                        if etx.send(Egress::Frame(Arc::new(f))).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Ok(ClientMsg::Pathfind { request_id, scene, start, waypoints, footprint_radius, token }) => {
                                    // One-shot pathfinding: resolve GM status, fetch explored off the lock for
                                    // non-GM Revealed, call SceneEcs::pathfind, reply to this connection only.
                                    // INVARIANT (one-shot-to-requester): reply goes to etx only, never broadcast.
                                    let req = PathfindRequest { request_id, scene, start, waypoints, footprint_radius, token };
                                    let frame = handle_pathfind(req, &ctx, &room, repo.as_ref()).await;
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

/// The `ClientMsg::Pathfind` frame's payload, carried as one value from the ingress match arm
/// into `handle_pathfind`.
///
/// INVARIANT: every field is CLIENT-SUPPLIED and unauthorized at construction. `handle_pathfind`
/// is where each earns trust: `scene` via the non-GM presence gate
/// (`SceneEcs::user_owns_token_in_scene`), `token` via the effective-ownership plus
/// same-scene-parent check, and `footprint_radius` only when no `token` is named — a named token
/// REPLACES it with `SceneEcs::resolve_token_footprint`'s value, so a route preview and the
/// authoritative gate cannot disagree about the mover's size.
///
/// This is deliberately NOT the same type as `scene::RouteRequester` or the parameters of
/// `SceneEcs::pathfind`, even though `scene`/`start`/`waypoints`/`footprint_radius` reach that
/// call: these are the pre-authorization values, and one shared type across the boundary would
/// let a caller forward the frame straight through, skipping the token-derived footprint override.
struct PathfindRequest {
    /// Correlation id echoed on the `PathResult`/`PathError` reply.
    request_id: Uuid,
    /// The scene to route in. Not proof of presence — see the struct INVARIANT.
    scene: Uuid,
    /// The mover's current position, the route's first point.
    start: (f64, f64),
    /// Ordered leg list whose last element is the goal.
    waypoints: Vec<(f64, f64)>,
    /// Hypothetical footprint radius in cells; IGNORED when `token` is `Some`.
    footprint_radius: f64,
    /// Optional footprint source. Authorized before use, and never a presence proof.
    token: Option<Uuid>,
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
async fn handle_pathfind(
    req: PathfindRequest,
    ctx: &crate::data::membership::PermissionContext,
    room: &crate::ws::room::Room,
    repo: &dyn crate::data::repository::Repository,
) -> ServerMsg {
    let PathfindRequest {
        request_id,
        scene,
        start,
        waypoints,
        footprint_radius,
        token,
    } = req;
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
    // The world's capability grants — an input to the movement-budget clamp's `cap::READ`
    // resolution (`budget_gate_for_token`), fetched ahead of any scene read guard exactly as
    // `Room::execute_move` fetches it (no await under a guard). Unconditional for the same
    // reason there: whether a combat is running is only knowable under the guard. Fails closed
    // like the executor — an unresolvable authority input refuses the preview generically.
    let world_defaults = match repo.world_cap_defaults(room.world_id).await {
        Ok(wd) => wd,
        Err(_) => {
            return ServerMsg::PathError {
                request_id,
                message: "unreachable".to_string(),
            };
        }
    };
    // Step 1: check movement_restriction under a short read guard, then drop it. The grid kind is
    // captured in the SAME guard from the `ResolvedScene` already being resolved, so the decode
    // below never re-acquires the lock for it.
    let (need_explored, grid_kind) = {
        let s = room.scene().read().await;
        let resolved = s.resolve_scene(scene);
        (
            !is_gm
                && matches!(
                    resolved.movement_restriction,
                    crate::scene::MovementRestriction::Revealed
                ),
            resolved.grid_kind,
        )
    };
    // Step 2: fetch explored (if needed) after the lock is dropped.
    let explored = if need_explored {
        match repo.get_explored(scene, ctx.user_id).await {
            Ok(Some(blob)) => Some(crate::scene::explored::ExploredSet::from_bytes(
                &blob, grid_kind,
            )),
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
                Some(_) => s.resolve_token_footprint(t, scene),
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
    // The movement-budget preview clamp, resolved through the SAME gate the executor uses
    // (`budget_gate_for_token` + `resolve_budget`) for a named, authorized token — a
    // hypothetical-footprint preview names no combatant identity and is never clamped.
    // `NotYourTurn`/`Unresolvable` mirror the executor's refusals behind the one generic
    // wording; `Resolved` yields a ceiling only for an enforced Hard caller, so GM/Warn/
    // None/exempt previews pass no budget and are untouched. The decrement half of a
    // resolution is ignored: a preview commits nothing.
    let mut budget_cells: Option<f64> = None;
    // The reply value disclosed as `PathResult.budget_cells`: present whenever the caller is
    // `enforced` on the named combatant (GM included), regardless of enforcement mode — distinct
    // from `budget_cells` above, which stays the `Hard`-only truncation ceiling `s.pathfind(..)`
    // consumes. The two locals must never share a binding: reusing `budget_cells` for the reply
    // would truncate `Warn`/`None` previews that must render in full.
    let mut reply_budget_cells: Option<f64> = None;
    if let Some(t) = token {
        if let Some(bg) = crate::ws::room::budget_gate_for_token(&s, scene, t, ctx, &world_defaults)
        {
            let enforced = bg.enforced();
            match crate::ws::room::resolve_budget(&bg, is_gm) {
                crate::ws::room::BudgetResolution::NotYourTurn
                | crate::ws::room::BudgetResolution::Unresolvable => {
                    return ServerMsg::PathError {
                        request_id,
                        message: "unreachable".to_string(),
                    };
                }
                crate::ws::room::BudgetResolution::Resolved {
                    budget_cells: b,
                    decrement,
                } => {
                    budget_cells = b;
                    if enforced {
                        reply_budget_cells = decrement.map(|d| d.resource_cells());
                    }
                }
            }
        }
    }
    match s.pathfind(
        crate::scene::RouteRequester {
            user: ctx.user_id,
            is_gm,
            explored: explored.as_ref(),
        },
        scene,
        start,
        &waypoints,
        footprint_radius,
        budget_cells,
    ) {
        Ok(outcome) => ServerMsg::PathResult {
            request_id,
            path: outcome.path,
            cost: outcome.cost,
            arrested: outcome.arrested,
            truncated: outcome.truncated,
            budget_cells: reply_budget_cells,
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
/// room via `Room::broadcast_aux_shared` (no seq, mirrors `ScenePing`). No success frame is
/// returned to the requester's `etx` — the broadcast IS the notification. The atomic position
/// `Event` from `commit_ops_locked` carries the authoritative position update for
/// document-store sync.
/// INVARIANT (no-geometry-leak): on any `execute_move` failure the reply is a generic
/// `MoveError { message: "move rejected" }` to `etx` only — no path geometry or vision state
/// is disclosed.
/// INVARIANT (mover_vision): `exec.frame`'s `ServerMsg::MoveStream.mover_vision` is `None` for
/// GM movers (no fog to sweep) and `Some` for player movers; the mapping to wire `VisionSample`
/// (per-polygon vertex capping, fail-closed under-reveal) lives in `Room::execute_move`'s frame
/// construction.
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
        .execute_move(
            repo,
            ctx,
            crate::ws::room::MoveRequestInputs {
                scene_id,
                token: token_id,
                path: path_tuples,
                ts: now,
                request_id,
            },
        )
        .await
    {
        Ok(exec) => {
            room.broadcast_aux_shared(exec.frame);
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
            Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob, shape.kind()),
            _ => crate::scene::explored::ExploredSet::new(),
        };
        if accumulate && set.mark_polygons(&scene_polys, shape, cell) > 0 {
            let _ = repo
                .set_explored(world, scene, user, &set.to_bytes(shape.kind()))
                .await;
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
/// INVARIANT (timeline-clip): each sample is judged against the clip target's vision AT THAT
///   SAMPLE'S INSTANT — the target's own in-flight `mover_vision` sweep (`Room::mover_streams`,
///   `ws::move_clip`) while one is active, the committed-position vision otherwise. The timeline
///   is the target's OWN vision (already sent to them), so this never admits a sample their fog
///   will not show. A target whose move starts AFTER this frame was clipped is served by the
///   egress re-emit (`egress_loop`'s own-move arm), not by this function.
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
    //   `scene` via `observer_vision_polys_for_scene` (committed position) and
    //   `Room::mover_streams` (in-flight timelines), both filtered/queried by that scene id. A
    //   see-as whose target has NO vision source in the move's scene (e.g. their token is in a
    //   different scene) yields zero committed polygons AND no in-flight timeline → the see-as
    //   does not apply → the GM keeps the full stream. A target WITH a source in the scene but no
    //   visible sample is suppressed, exactly like a real observer.
    // Whose vision this recipient is clipped against: their own, or (a GM see-as) the target's.
    let target_user = if ctx.world_role == crate::data::document::WorldRole::Gm {
        match see_as {
            Some(target) => target.user_id,
            None => return Some(full_gm_stream()),
        }
    } else {
        ctx.user_id
    };
    let now = crate::ws::time::now_millis();
    // Committed-position vision (the at-rest gate) and the target's in-flight sweep timelines.
    // Both reads drop their locks before this function's caller awaits `sink.send`.
    let static_polys = observer_vision_polys_for_scene(target_user, *scene, room).await;
    let timeline_frames = room.mover_streams(target_user, *scene, now).await;
    // A registered stream carries a usable timeline only when its own frame has
    // `mover_vision: Some(_)` — a GM mover's own executed move always registers with
    // `mover_vision: None` (`Room::execute_move`), so `timeline_frames` can be non-empty while
    // containing nothing this clip can use. The applicability check below MUST test this
    // filtered set, not the raw registered-stream count.
    let timelines: Vec<crate::ws::move_clip::TimelineStream<'_>> = timeline_frames
        .iter()
        .filter_map(|f| match f.as_ref() {
            ServerMsg::MoveStream {
                start_server_ms,
                mover_vision: Some(v),
                ..
            } => Some(crate::ws::move_clip::TimelineStream {
                start_server_ms: *start_server_ms,
                vision: v,
            }),
            _ => None,
        })
        .collect();
    if ctx.world_role == crate::data::document::WorldRole::Gm
        && static_polys.is_empty()
        && timelines.is_empty()
    {
        // See-as target has no vision source in this scene → not applicable → full GM stream.
        return Some(full_gm_stream());
    }
    let visible =
        crate::ws::move_clip::clip_samples(samples, *start_server_ms, &static_polys, &timelines);
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
    cache: &Arc<crate::modules::ModuleScanCache>,
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
        let cache = cache.clone();
        let installed = tokio::task::spawn_blocking(move || cache.get_or_scan(&dir))
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

/// What a connection knows about its world, fixed for the connection's
/// duration: the room it publishes/subscribes through, the document
/// repository, the caller's resolved identity and role, the resync watermark
/// `egress_loop` starts from, the installed-modules directory used to
/// build the Welcome capability requirements, and the shared module-scan
/// cache backing it.
struct EgressConnState {
    /// The world's room — the authoritative publish/subscribe path.
    room: Arc<Room>,
    /// The document repository.
    repo: Arc<SqliteRepository>,
    /// The caller's authenticated identity and world role, resolved once at
    /// connect time and reused for every outgoing frame's per-recipient filter.
    ctx: PermissionContext,
    /// The resync watermark `egress_loop` starts delivering from.
    current_seq: i64,
    /// The installed-modules directory scanned for the Welcome frame's
    /// capability requirements.
    modules_dir: std::path::PathBuf,
    /// Shared cache for the installed-module scan behind the Welcome frame's
    /// capability requirements — see `crate::modules::ModuleScanCache`.
    module_scan_cache: Arc<crate::modules::ModuleScanCache>,
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
async fn egress_loop<S>(
    mut sink: S,
    mut rx: tokio::sync::broadcast::Receiver<crate::ws::room::RoomEvent>,
    mut erx: mpsc::Receiver<Egress>,
    conn: EgressConnState,
) where
    S: Sink<Message> + Unpin,
{
    let EgressConnState {
        room,
        repo,
        ctx,
        current_seq,
        modules_dir,
        module_scan_cache,
    } = conn;
    let world_id = room.world_id;
    // Loaded once per connection (not per event): a per-event read would contend
    // with apply_intent on the single-writer pool. A defaults change mid-session
    // takes effect on the client's next (re)connect.
    let world_defaults = repo.world_cap_defaults(world_id).await.unwrap_or_default();
    // Fail open for the advisory client copy only; server-side enforcement
    // reads requirements freshly per intent and fails closed.
    let world_reqs =
        welcome_capability_requirements(repo.as_ref(), world_id, &modules_dir, &module_scan_cache)
            .await;
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
        // `biased` — `erx` (this connection's own direct replies, incl. `Reject`) MUST be
        // drained ahead of `rx` (the room broadcast, incl. this same connection's own
        // self-authored `Event` echoes) whenever both are ready. `Room::publish` serializes
        // intents one at a time (`publish_guard`): an intent's `Reject` is enqueued onto `erx`
        // and fully sent before the NEXT intent's `publish()` call — which produces any LATER
        // broadcast `Event` — even starts, so insertion order is always
        // `erx(reject_1) < erx(reject_2) < rx(event_3)` for a client's own back-to-back
        // intents. An unbiased `select!` picks among ready branches with no ordering
        // guarantee, so it can deliver `event_3` before `reject_1`/`reject_2` even though the
        // server processed them strictly in order — `OptimisticClient.applyCommand`'s
        // confirm-on-self-authored-echo is a byte-for-byte FIFO shift (see its own doc), and a
        // single such inversion permanently misaligns every later self-authored confirm for
        // the rest of the connection's lifetime, since the shift silently confirms whatever
        // pending entry is oldest rather than the one the arriving command actually completes.
        tokio::select! {
            biased;
            cmd = erx.recv() => match cmd {
                Some(Egress::Frame(f)) => {
                    if send_plain(&mut sink, f.as_ref()).await.is_err() { break; }
                }
                Some(Egress::TimePong { client_t0, server_t }) => {
                    if sink.send(text(&ServerMsg::TimePong { client_t0, server_t })).await.is_err() { break; }
                }
                Some(Egress::Resync(from)) => {
                    // Clamp an EXPLICIT client-driven resync against this user's floor —
                    // closes the "any member can request the entire world history
                    // unvalidated" gap. Deliberately NOT applied to the `Lagged`-driven
                    // auto-resync below: that path replays from this connection's own
                    // live-tracked `next_expected` watermark, not an untrusted
                    // client-supplied `from_seq`, so it is not the reachability gap this
                    // clamp exists for. Gated by `Room::resync_floor_enforced` — see its
                    // doc for why production leaves it off until the client sends a
                    // cold-start `Hello`.
                    let from = if room.resync_floor_enforced() {
                        from.max(room.resync_floor(ctx.user_id).await)
                    } else {
                        from
                    };
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
                            (crate::scene::compute_derived(&channel, &ecs, &view_ctx, &world_defaults), ecs.committed_seq(), ecs.scene_grid_sizes(), ecs.scene_grid_shapes())
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
                        if send_room_event(&mut sink, repo.as_ref(), &room, &ctx, &world_defaults, &msg).await.is_err() { break; }
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
                        let crate::ws::room::RoomEvent::Other(inner) = &msg else {
                            unreachable!("event_seq() is Some for every RoomEvent::Event");
                        };
                        let should_break = match inner.as_ref() {
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
                                let mut failed = match clip_move_stream(inner.as_ref(), &ctx, see_as, &room).await {
                                    Some(out) => sink.send(text(&out)).await.is_err(),
                                    None => false, // suppressed: do not send
                                };
                                // Own-move re-emit: the clip target's vision timeline just changed, so every
                                // OTHER in-flight stream in this scene is re-clipped against it and re-sent
                                // under its original token_id (the client overwrites playback keyed by
                                // token_id in place). Serves the ordering where the recipient's move starts
                                // AFTER the other stream was clipped — the clip itself cannot widen a frame
                                // already sent. Delivered only to this connection; other recipients'
                                // timelines are unchanged. Requires `mover_vision: Some(_)` on the
                                // triggering frame: a zero-progress move (never registered in `moving`, so
                                // never a source of concurrent re-clips) and a GM mover's own move
                                // (`mover_vision: None`, so it's always filtered out of a target's usable
                                // timeline regardless) cannot have produced a timeline anything could
                                // meaningfully re-clip against — skipping them avoids driving the re-emit
                                // loop at request rate off a move that could never populate one.
                                if !failed {
                                    if let ServerMsg::MoveStream {
                                        mover,
                                        scene,
                                        mover_vision: Some(_),
                                        ..
                                    } = inner.as_ref()
                                    {
                                        let clip_target = see_as.map(|t| t.user_id).unwrap_or(ctx.user_id);
                                        if *mover == clip_target {
                                            let now = crate::ws::time::now_millis();
                                            for other in room.concurrent_streams(*scene, clip_target, now).await {
                                                if let Some(out) =
                                                    clip_move_stream(other.as_ref(), &ctx, see_as, &room).await
                                                {
                                                    if sink.send(text(&out)).await.is_err() {
                                                        failed = true;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                failed
                            }
                            ServerMsg::Evicted { user } => {
                                // Targeted eviction. Delivery of the frame is
                                // best-effort; the Close and the `break` are the
                                // point — the ingress loop tears the connection
                                // down when this egress task exits.
                                if user.is_none() || *user == Some(ctx.user_id) {
                                    let _ = sink.send(text(inner.as_ref())).await;
                                    let _ = sink.send(Message::Close(None)).await;
                                    true
                                } else {
                                    false
                                }
                            }
                            other => send_plain(&mut sink, other).await.is_err(),
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
                            crate::scene::compute_derived(&s.channel, &ecs, &s.view_ctx, &world_defaults),
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
/// watermark matches; events after `to_seq` arrive via normal live delivery. Callers that
/// need `from_seq` clamped against a resync floor (the `Egress::Resync` handler) must do
/// so BEFORE calling this — the `Lagged`-driven auto-resync call site deliberately clamps
/// nothing, since it replays from this connection's own live-tracked watermark, not an
/// untrusted client-supplied `from_seq`.
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
    for f in &frames {
        send_room_event(sink, repo, room, ctx, world_defaults, f).await?;
    }
    sink.send(text(&ServerMsg::ResyncEnd {
        current_seq: to_seq,
    }))
    .await
    .map_err(|_| ())?;
    Ok(to_seq)
}

/// Bring `room`'s world-config singleton set current under
/// `WriteOrigin::ConfigSeed`: creates whatever `missing_config_ops` finds
/// absent and refreshes a drifted `system-defaults` body from the enabled
/// system package. Attributed to the world's first GM (`seed_author`); a
/// world with no GM is a no-op Ok. A `Conflict` from a lost seed race (a
/// concurrent join seeded first) is swallowed — the winner's docs are live
/// and this pass has nothing left to do; any other error propagates for the
/// caller to log (a join must never fail on a reseed).
pub(crate) async fn reseed_world_config(
    room: &Room,
    repo: &SqliteRepository,
    modules_dir: &std::path::Path,
) -> Result<(), crate::data::DataError> {
    let world_id = room.world_id;
    let Some(ctx) = crate::data::world_seed::seed_author(repo, world_id).await else {
        return Ok(());
    };
    let sd = crate::data::world_seed::enabled_system_defaults(repo, world_id, modules_dir).await;
    let types: Vec<&str> = crate::data::world_seed::CONFIG_SINGLETON_DOC_TYPES.to_vec();
    let existing = repo.query_documents_by_types(world_id, &types).await?;
    let ops =
        crate::data::world_seed::missing_config_ops(&existing, world_id, sd.as_ref(), now_millis());
    if ops.is_empty() {
        return Ok(());
    }
    match room
        .publish(repo, &ctx, ops, now_millis(), WriteOrigin::ConfigSeed)
        .await
    {
        Ok(_) => Ok(()),
        Err(crate::data::DataError::Conflict(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests;
