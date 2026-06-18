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

use crate::auth::session::AuthUser;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::http::AppState;
use crate::ws::protocol::{ClientMsg, ServerMsg, WsErrorCode};
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
    ws.on_upgrade(move |socket| handle_socket(socket, state, user.id, q.world))
}

/// Serialize a server frame to a text WS message. Serializing our own types
/// never fails.
fn text(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn handle_socket(socket: WebSocket, state: AppState, user_id: Uuid, world_id: Uuid) {
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
        world_id,
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
                        Ok(ClientMsg::EmitTest { .. }) => {
                            if let Err(e) = room.publish(repo.as_ref(), user_id, now_millis()).await {
                                tracing::warn!(?e, "publish failed");
                                let _ = etx
                                    .send(Egress::Frame(Arc::new(ServerMsg::Error {
                                        code: WsErrorCode::PublishFailed,
                                        message: "publish failed".into(),
                                    })))
                                    .await;
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
    world_id: Uuid,
    current_seq: i64,
) where
    S: Sink<Message> + Unpin,
{
    if sink
        .send(text(&ServerMsg::Welcome {
            world: world_id,
            current_seq,
            server_time: now_millis(),
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
                    if sink.send(text(&f)).await.is_err() { break; }
                }
                Some(Egress::TimePong { client_t0, server_t }) => {
                    if sink.send(text(&ServerMsg::TimePong { client_t0, server_t })).await.is_err() { break; }
                }
                Some(Egress::Resync(from)) => {
                    if replay(&mut sink, &room, repo.as_ref(), from).await.is_err() { break; }
                    next_expected = room.current_seq() + 1;
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
                            if replay(&mut sink, &room, repo.as_ref(), next_expected).await.is_err() { break; }
                            next_expected = room.current_seq() + 1;
                            if seq < next_expected { continue; }
                        }
                        if sink.send(text(&msg)).await.is_err() { break; }
                        next_expected = seq + 1;
                    } else if sink.send(text(&msg)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    room.stats.lagged_drops.fetch_add(n, Ordering::Relaxed);
                    tracing::warn!(world = %world_id, dropped = n, "broadcast lagged");
                    if replay(&mut sink, &room, repo.as_ref(), next_expected).await.is_err() { break; }
                    next_expected = room.current_seq() + 1;
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Replay `[from_seq, current]` to the sink as ResyncBegin .. Event* .. ResyncEnd.
async fn replay<S>(sink: &mut S, room: &Room, repo: &dyn Repository, from_seq: i64) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    let (frames, source) = room.resync_range(repo, from_seq).await.map_err(|_| ())?;
    let to_seq = frames.last().and_then(|m| m.event_seq()).unwrap_or(from_seq - 1);
    tracing::debug!(from_seq, to_seq, ?source, "resync served");
    sink.send(text(&ServerMsg::ResyncBegin { from_seq, to_seq, source }))
        .await
        .map_err(|_| ())?;
    for f in frames {
        sink.send(text(&f)).await.map_err(|_| ())?;
    }
    sink.send(text(&ServerMsg::ResyncEnd { current_seq: room.current_seq() }))
        .await
        .map_err(|_| ())?;
    Ok(())
}
