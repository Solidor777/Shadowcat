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
use crate::data::document::CapabilityGrants;
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
    TimePong { client_t0: i64, server_t: i64 },
    Resync(i64),
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
    world_defaults: &CapabilityGrants,
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
                        Ok(ClientMsg::Search { request_id, query, limit, cursor, subscribe: _ }) => {
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
                        Ok(ClientMsg::Unsubscribe { .. }) => {}
                        Ok(ClientMsg::Hello { .. }) | Ok(ClientMsg::Pong) => {}
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
    // Project the world grants to only what this actor needs to self-gate; other
    // users' UUIDs and grants must not cross to the client.
    let actor_grants = crate::data::permission::project_grants_for(&world_defaults, ctx.user_id);
    if sink
        .send(text(&ServerMsg::Welcome {
            world: world_id,
            current_seq,
            server_time: now_millis(),
            world_default_grants: actor_grants,
            actor_role: ctx.world_role,
            capability_requirements: world_reqs,
        }))
        .await
        .is_err()
    {
        return;
    }

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
                    } else if send_filtered(&mut sink, repo.as_ref(), &ctx, &world_defaults, msg.as_ref()).await.is_err() {
                        break;
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
    world_defaults: &CapabilityGrants,
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
