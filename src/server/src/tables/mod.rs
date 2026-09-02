//! Server-side rollable-table draws: `handle_draw_table` resolves one or
//! more draws from a `table` document and posts the result to chat as ONE
//! `MessageKind::Roll` message, through the exact same `build_message_doc` +
//! `Room::publish` chokepoint every other message goes through. `draw::
//! draw_table` does the actual recursive per-draw resolution.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub(crate) mod draw;

use uuid::Uuid;

use crate::chat::{
    self, ActorOwnerRef, Audience, MessageDraft, MessageKind, Segment, SendMessageError,
};
use crate::data::command::{Command, Operation, WriteOrigin};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::room::Room;
use crate::ws::PingRateLimiter;

/// A top-level `DrawTable` request's own `count` cap (before any nested
/// `TableEntry::Draw` fan-out is even considered).
pub(crate) const MAX_TOP_LEVEL_DRAWS: u32 = 10;
/// Every draw resolved across one request -- top-level plus every nested
/// draw fanned out from a matched row -- counts against this bound.
pub(crate) const MAX_DRAWS_PER_REQUEST: usize = 64;
/// Maximum recursion depth a nested `TableEntry::Draw` chain may reach.
pub(crate) const MAX_DRAW_DEPTH: usize = 8;

/// Fixed parse context for every table roll: Total mode (a table matches one
/// total against a row's range/cumulative weight, never a success count) and
/// `HighWins` -- channel-independent, unlike an ordinary chat roll's ambient
/// `dice-settings` resolution (a table's outcome must not depend on which
/// channel it happened to be drawn into).
pub(crate) use crate::chat::rolls::TABLE_PARSE_CONTEXT;

/// Why a `DrawTable` request was refused. `[sec]`-classified `Display`
/// mirrors `SendMessageError`'s own rule: `Forbidden`/`NotFound`/`Data`
/// collapse to one generic, existence-hiding string; every other variant is
/// specific and player-presentable.
#[derive(Debug)]
pub enum DrawTableError {
    /// The caller's per-minute chat flood budget is exhausted.
    RateLimited,
    /// `channel` is not a key of the world's channel registry.
    UnknownChannel,
    /// An `Audience::Whisper` recipient does not belong to this world, or the
    /// recipient list exceeds `chat::MAX_WHISPER_RECIPIENTS`.
    UnknownRecipient,
    /// The caller may not attribute this draw to the named actor/token.
    ActorNotSpeakable,
    /// The caller lacks READ on a table somewhere in the draw chain.
    Forbidden,
    /// A table id in the chain does not resolve to a `table` document in
    /// this world.
    NotFound,
    /// The request (or a nested fan-out) would resolve more than
    /// `MAX_DRAWS_PER_REQUEST` draws.
    TooMany,
    /// A nested `TableEntry::Draw` chain exceeded `MAX_DRAW_DEPTH`.
    TooDeep,
    /// A table names itself, directly or through a chain of nested draws.
    Cycle,
    /// A table's `rows` is empty.
    EmptyTable,
    /// A `TableEntry::Image` names an asset absent from, or outside, the
    /// drawing world -- a GM-fixable table-authoring error.
    MissingAsset,
    /// The table's roll failed to parse or exceeded a wire-boundary cap.
    Roll(chat::rolls::RollError),
    /// A repository error.
    Data(crate::data::DataError),
    /// An `Audience::Whisper`'s recipient list exceeded
    /// `chat::MAX_WHISPER_RECIPIENTS`, mirroring `SendMessageError::TooLong`'s
    /// whisper-cap case (the only `TooLong` case `validate_audience` can
    /// produce -- content-length caps don't apply to a draw, which has no
    /// author-typed body).
    TooLong,
}

impl std::fmt::Display for DrawTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrawTableError::RateLimited => write!(
                f,
                "You are sending messages too quickly. Please wait a moment."
            ),
            DrawTableError::UnknownChannel => write!(f, "That channel does not exist."),
            DrawTableError::UnknownRecipient | DrawTableError::TooLong => {
                write!(f, "One or more whisper recipients could not be resolved.")
            }
            DrawTableError::ActorNotSpeakable => {
                write!(f, "You are not permitted to send this message.")
            }
            DrawTableError::Forbidden | DrawTableError::NotFound | DrawTableError::Data(_) => {
                write!(f, "That table could not be found.")
            }
            DrawTableError::TooMany => write!(f, "That draw is too large."),
            DrawTableError::TooDeep => write!(f, "That draw nests too deeply."),
            DrawTableError::Cycle => write!(f, "That table refers back to itself."),
            DrawTableError::EmptyTable => write!(f, "That table has no rows."),
            DrawTableError::MissingAsset => {
                write!(f, "That table references an image that could not be found.")
            }
            DrawTableError::Roll(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DrawTableError {}

impl From<SendMessageError> for DrawTableError {
    /// Maps the two `SendMessageError`-returning functions `handle_draw_table`
    /// reuses (`chat::validate_audience`/`chat::validate_actor_owner`) onto
    /// this error type. Every OTHER `SendMessageError` variant is not
    /// producible by either function; mapped conservatively to `Forbidden`
    /// (never leaks anything) so a future `SendMessageError` variant compiles
    /// here rather than panicking in production.
    fn from(e: SendMessageError) -> Self {
        match e {
            SendMessageError::TooLong => DrawTableError::TooLong,
            SendMessageError::UnknownRecipient => DrawTableError::UnknownRecipient,
            SendMessageError::ActorNotSpeakable => DrawTableError::ActorNotSpeakable,
            SendMessageError::Data(e) => DrawTableError::Data(e),
            SendMessageError::Empty
            | SendMessageError::RateLimited
            | SendMessageError::UnknownChannel
            | SendMessageError::NotFound
            | SendMessageError::Forbidden
            | SendMessageError::AudienceLocked
            | SendMessageError::Roll(_)
            | SendMessageError::RollImmutable => DrawTableError::Forbidden,
        }
    }
}

/// Borrowed dependencies `handle_draw_table` needs, the `MessageRequestCtx`
/// shape.
pub struct DrawTableRequestCtx<'a> {
    /// The world's room -- the authoritative publish path.
    pub room: &'a Room,
    /// The document repository.
    pub repo: &'a dyn Repository,
    /// The caller's authenticated identity and world role.
    pub ctx: &'a PermissionContext,
    /// The per-user chat flood-budget limiter.
    pub rate: &'a PingRateLimiter,
    /// The moment of this request.
    pub now: i64,
    /// The per-user-per-minute flood budget (the same one chat sends spend).
    pub budget_per_min: usize,
}

/// Dispatches one `DrawTable` request: flood-limit, validate channel/
/// audience/attribution (the same chokepoints `handle_send_message` uses),
/// resolve `count` top-level draws, and publish ONE `MessageKind::Roll`
/// message via the sole authoring path every message goes through.
pub(crate) async fn handle_draw_table(
    req: DrawTableRequestCtx<'_>,
    table_id: Uuid,
    channel: String,
    count: u32,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
) -> Result<Command, DrawTableError> {
    let DrawTableRequestCtx {
        room,
        repo,
        ctx,
        rate,
        now,
        budget_per_min,
    } = req;

    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(DrawTableError::RateLimited);
    }
    if !chat::channel_registered(repo, room.world_id, &channel)
        .await
        .map_err(DrawTableError::Data)?
    {
        return Err(DrawTableError::UnknownChannel);
    }
    chat::validate_audience(repo, room.world_id, ctx.user_id, &audience).await?;
    if let Some(owner) = &actor_owner {
        chat::validate_actor_owner(repo, room, ctx, owner).await?;
    }
    if !(1..=MAX_TOP_LEVEL_DRAWS).contains(&count) {
        return Err(DrawTableError::TooMany);
    }

    // Loaded internally (not caller-supplied), mirroring
    // `combat::handle_combat_intent`'s own acquisition -- a fresh read per
    // request, not a connection-lifetime cache.
    let world_defaults = repo
        .world_cap_defaults(room.world_id)
        .await
        .map_err(DrawTableError::Data)?;
    let policy = chat::resolve_content_policy(repo, room.world_id).await;
    let mut cx = draw::DrawCtx {
        repo,
        ctx,
        world_defaults: &world_defaults,
        policy: &policy,
        world_id: room.world_id,
        chain: Vec::new(),
        budget: 0,
    };
    let mut content: Vec<Segment> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let seg = draw::draw_table(&mut cx, table_id, 0).await?;
        content.push(Segment::TableDraw(seg));
    }

    let doc = chat::build_message_doc(
        room.world_id,
        ctx.user_id,
        MessageDraft {
            channel,
            actor_owner,
            audience,
            kind: MessageKind::Roll,
            content,
            source: None,
        },
        now,
    );
    room.publish(
        repo,
        ctx,
        vec![Operation::Create { doc }],
        now,
        WriteOrigin::Client,
    )
    .await
    .map_err(DrawTableError::Data)
}

#[cfg(test)]
mod tests;
