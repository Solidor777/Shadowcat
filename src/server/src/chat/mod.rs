//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with an opaque `system` body
//! (this module's `MessageSystem`), authored and revised ONLY by the server —
//! never built or mutated by a client directly. A `message` doc_type reaches
//! `apply_intent` only via `handle_send_message` (Create), `handle_edit_message`,
//! or `handle_delete_message` (both Update, the latter a soft tombstone). Four
//! chokepoints jointly enforce this: the create-gate baseline-message exemption
//! (`sqlite.rs`, ties a Create to its authenticated author); the ingress guard
//! (`ops_target_message`) rejects any client-authored `message` Create/Delete op
//! at the WS/HTTP boundary; `apply_intent`'s `Update` branch blanket-rejects
//! every client (`WriteOrigin::Client`) Update targeting a stored `message` doc
//! (Updates carry no `doc_type`, so they cannot be classified by
//! `ops_target_message` and must be blocked against the authoritative stored
//! document instead), exempting ONLY `WriteOrigin::ServerMessageRevision` — a
//! marker no wire frame can set, produced solely by `handle_edit_message`/
//! `handle_delete_message` after their own owner-or-GM check — and granting it a
//! scoped `Access` (`READ`+`WRITE_FIELDS` only, never `/permissions`/`/embedded`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

mod commands;
mod sanitize;
mod settings;
mod shortcodes;
pub use commands::{parse_command, ParsedCommand};
pub use sanitize::sanitize;
pub use settings::{resolve_content_policy, ChatContentPolicy, CHAT_SETTINGS_DOC_TYPE};

use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::ws::room::Room;
use crate::ws::PingRateLimiter;

/// Top-level doc_type for chat messages.
pub const MESSAGE_DOC_TYPE: &str = "message";

/// True if any op authors a `message` doc via the generic document path.
/// Clients must NOT author messages (only `handle_send_message` may); the WS
/// `Intent` and HTTP write paths reject ops for which this is true, keeping
/// message ingest server-authoritative.
///
/// `Operation::Update` carries no `doc_type` (just `doc_id` + field changes),
/// so it cannot be classified here; every `WriteOrigin::Client` `Update`
/// targeting a stored `message` doc is instead rejected in `apply_intent`'s
/// `Update` branch (classified there against the authoritative stored
/// `doc_type`). That branch exempts ONLY `WriteOrigin::ServerMessageRevision`
/// — the legitimate message-edit/-delete path introduced in c-3
/// (`handle_edit_message`/`handle_delete_message`), unreachable from any
/// client transport.
pub fn ops_target_message(ops: &[Operation]) -> bool {
    ops.iter().any(|op| match op {
        Operation::Create { doc } | Operation::Delete { doc } => doc.doc_type == MESSAGE_DOC_TYPE,
        Operation::Update { .. } => false,
    })
}

/// Attribution of a message to an actor: a linked canonical `Actor` document,
/// or an instanced actor resolved through its token. Carried on the
/// `SendMessage` frame and stored in `MessageSystem`. No ID newtypes exist —
/// identifiers are bare `Uuid` (rendered `string` in TS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorOwnerRef {
    Actor { actor_id: Uuid },
    TokenInstance { token_id: Uuid },
}

/// The intended readership of a message, beyond the ordinary world-readable
/// default. Carried on the `SendMessage` frame and stored verbatim in
/// `MessageSystem`; drives the document's `PermissionSet` in
/// `build_message_doc` (see that function for the exact mapping). `channel`
/// stays a purely client-chosen label — the server never validates it or
/// derives audience from it; a client module choosing to post into a "GM"
/// channel is what sets `Audience::GmOnly`, not the channel string itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Audience {
    /// Every world member may read (c-1's original, unrestricted shape).
    #[default]
    Public,
    /// Only `recipients` (plus the sender) may read. The GM reads it ONLY if
    /// their own uuid is among `recipients` — not automatically.
    Whisper { recipients: Vec<Uuid> },
    /// Only whoever currently holds `WorldRole::Gm` (plus the sender) may
    /// read — resolved dynamically, not a frozen roster at send time.
    GmOnly,
}

/// Message subtype, orthogonal to channel. Rides the opaque body (no ts-rs).
/// c-1 only ever produces `Normal`; `Emote`/`Roll` are set by c-3's command
/// parser, `System` by server-authored notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Normal,
    Emote,
    Roll,
    System,
}

/// One piece of a message's sanitized content model. Serialized into the
/// message's opaque `system` body (no ts-rs — M11d declares its own Zod mirror).
/// Extensible: later checkpoints add the variants they produce (c-3 marks/links/
/// images, c-4 preview cards, M11d roll embeds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text. Rendered as a DOM text node by the client (never innerHTML),
    /// so any markup it contains is inert.
    Text { text: String },
    /// A run of ammonia-sanitized HTML (safe by construction; the client renders
    /// it via innerHTML). Produced only by `sanitize` (chat/sanitize.rs).
    Html { sanitized_html: String },
    // Reserved, produced later: RollEmbed (M11d), PreviewCard (c-4), DocLink (M11d).
}

/// The c-1 producer: wrap raw input as a single literal-text segment. Rich
/// producers (markdown/HTML) are added in c-3, feeding this same content model.
pub fn plain_text_content(raw: &str) -> Vec<Segment> {
    vec![Segment::Text {
        text: raw.to_string(),
    }]
}

/// The message document's `system` body. Opaque on the wire (no ts-rs); the
/// client declares its own Zod mirror in M11d.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSystem {
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    pub kind: MessageKind,
    #[serde(default)]
    pub audience: Audience,
    pub content: Vec<Segment>,
    /// The author's raw input (post-`/w`-strip), kept for client edit-prefill —
    /// sanitized `Segment::Html` cannot be reversed to author input. Data only,
    /// never rendered as markup. MUST be cleared by the delete tombstone with
    /// `content` (a retained source would leak deleted content).
    ///
    /// EXPOSURE NOTE: like every string leaf of `system` (incl. `channel`),
    /// this pre-sanitize text is swept into the content-agnostic FTS index and
    /// can surface in `SearchHit.snippet`/`.document`. Any search-UI consumer
    /// must treat message-doc snippet/`source` strings as inert text (never
    /// innerHTML) — this field is the highest-volume raw-text instance of that
    /// pre-existing pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Set when the message has been edited (c-3's edit path). Absent (not
    /// `null`) on the wire for an unedited message, so a stored c-1 message
    /// with no marker still round-trips unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    /// Set when the message has been soft-deleted (c-3's delete path). Absent
    /// (not `null`) on the wire for a live message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
}

/// Server-construct a message `Document`. INVARIANT: only the server calls
/// this (via `handle_send_message`); clients never build message docs.
/// `audience` drives the document's `PermissionSet`:
/// - `Public` — `default: Observer`, `gm_role: None` (c-1's original,
///   world-readable shape; the GM's unconditional access is unaffected).
/// - `Whisper { recipients }` — `default: None`, `gm_role: Some(None)` (the
///   GM reads only if individually listed), `users` holds `owner: Owner` plus
///   each recipient as `Observer`.
/// - `GmOnly` — `default: None`, `gm_role: Some(Observer)` (ANY current GM
///   reads, resolved dynamically — not a frozen roster), `users` holds only
///   `owner: Owner`.
///
/// In every case `owner` is inserted into `users` LAST, so a `Whisper` that
/// redundantly names the sender as their own recipient can never downgrade
/// them from `Owner` to `Observer` via map-insertion order.
#[allow(clippy::too_many_arguments)]
pub fn build_message_doc(
    world_id: Uuid,
    user: Uuid,
    channel: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    kind: MessageKind,
    content: Vec<Segment>,
    source: Option<String>,
    now: i64,
) -> Document {
    let (default, gm_role, mut users) = match &audience {
        Audience::Public => (DocRole::Observer, None, BTreeMap::new()),
        Audience::Whisper { recipients } => {
            let mut users = BTreeMap::new();
            for &r in recipients {
                if r != user {
                    users.insert(r, DocRole::Observer);
                }
            }
            (DocRole::None, Some(DocRole::None), users)
        }
        Audience::GmOnly => (DocRole::None, Some(DocRole::Observer), BTreeMap::new()),
    };
    users.insert(user, DocRole::Owner);
    let system = MessageSystem {
        channel,
        user_owner: user,
        actor_owner,
        kind,
        audience,
        content,
        source,
        edited_at: None,
        deleted_at: None,
    };
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        source: None,
        owner: Some(user),
        permissions: PermissionSet {
            default,
            users,
            gm_role,
            ..Default::default()
        },
        embedded: BTreeMap::new(),
        parent_id: None,
        system: serde_json::to_value(system).expect("MessageSystem serializes"),
        created_at: now,
        updated_at: now,
    }
}

/// Max characters accepted for a single message's raw content (pre-producer).
pub const MAX_MESSAGE_CHARS: usize = 4096;

/// Max characters accepted for a message's `channel` name. Otherwise `channel`
/// is unbounded save for the 256 KB whole-document size cap.
pub const MAX_CHANNEL_CHARS: usize = 128;

/// Max recipients accepted on an `Audience::Whisper`. A world's realistic
/// member count is small; this is generous but bounded — without it, a single
/// cheap `SendMessage` frame could force `handle_send_message` to run one
/// sequential `repo.member_role` DB round-trip PER recipient before the
/// message is even constructed, a resource-amplification availability risk.
pub const MAX_WHISPER_RECIPIENTS: usize = 128;

/// Why `handle_send_message` refused to ingest a `SendMessage` frame.
#[derive(Debug)]
pub enum SendMessageError {
    /// Content is empty after trimming whitespace, or `channel` is empty
    /// after trimming whitespace.
    Empty,
    /// Content exceeds `MAX_MESSAGE_CHARS`, `channel` exceeds
    /// `MAX_CHANNEL_CHARS`, or an `Audience::Whisper`'s `recipients` exceeds
    /// `MAX_WHISPER_RECIPIENTS`. Reused for all three — the surface stays
    /// minimal since neither the caller nor the wire protocol distinguishes
    /// which field/limit was exceeded.
    TooLong,
    /// The user's per-minute flood budget is exhausted.
    RateLimited,
    /// An `Audience::Whisper` recipient uuid does not belong to this world.
    /// Fail-closed: the whole send is rejected, nothing is persisted.
    UnknownRecipient,
    /// The authoritative write (`Room::publish`) failed.
    Data(DataError),
    /// The target message does not exist (edit/delete).
    NotFound,
    /// The requester is neither the message owner nor a GM (edit/delete).
    Forbidden,
    /// An edit attempted to change audience (a `/w` inside an edit). Frozen.
    AudienceLocked,
}

/// Server-authoritative message ingest: flood-limit, validate, CONSTRUCT the
/// message doc, and publish it via the authoritative path. The sole message-
/// authoring entry point (see module-level INVARIANT comment) — a client can
/// only ever reach a stored `message` doc through this function.
#[allow(clippy::too_many_arguments)]
pub async fn handle_send_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    channel: String,
    content: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if content.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if channel.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if channel.chars().count() > MAX_CHANNEL_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }
    // Parse leading command (server-authoritative kind; /w whisper targets).
    let parsed = parse_command(&content);
    // Cap the RAW /w name list BEFORE resolving a single username — resolving
    // first would run one sequential `member_id_by_username` DB round-trip per
    // `@name` token ahead of the cap check, reproducing the exact unbounded
    // per-recipient resource-amplification risk `MAX_WHISPER_RECIPIENTS` exists
    // to prevent (see that constant's doc comment).
    if let Some(names) = &parsed.whisper_to {
        if names.len() > MAX_WHISPER_RECIPIENTS {
            return Err(SendMessageError::TooLong);
        }
    }
    // Captured BEFORE `parsed.whisper_to` is consumed below — drives the
    // /w-prefix strip on `source` (see build_message_doc call).
    let had_whisper = parsed.whisper_to.is_some();
    // Effective audience: an explicit /w wins; otherwise the c-2 frame field.
    let audience = if let Some(names) = parsed.whisper_to {
        let mut recipients = Vec::with_capacity(names.len());
        for name in &names {
            match repo
                .member_id_by_username(room.world_id, name)
                .await
                .map_err(SendMessageError::Data)?
            {
                Some(uid) => recipients.push(uid),
                None => return Err(SendMessageError::UnknownRecipient),
            }
        }
        Audience::Whisper { recipients }
    } else {
        audience
    };
    // Re-validate the EFFECTIVE audience (whisper cap + membership) — the
    // single chokepoint covering BOTH the frame's `audience` field and a
    // content-level `/w` command, so neither front-door can bypass the cap
    // or the fail-closed unknown-recipient rejection. The cap is ALSO checked
    // above (pre-resolution) for the content-`/w` path specifically; this
    // second check is what actually guards the frame's `audience` argument
    // (which never runs the pre-check above) and stays authoritative for both.
    if let Audience::Whisper { recipients } = &audience {
        if recipients.len() > MAX_WHISPER_RECIPIENTS {
            return Err(SendMessageError::TooLong);
        }
        for &r in recipients {
            let is_member = repo
                .member_role(room.world_id, r)
                .await
                .map_err(SendMessageError::Data)?
                .is_some();
            if !is_member {
                return Err(SendMessageError::UnknownRecipient);
            }
        }
    }
    // A command that leaves no message body (e.g. `/w @alice` with no
    // trailing text) must be rejected the same way empty raw content is —
    // the top-level `content.trim().is_empty()` check above only guards the
    // PRE-parse raw string, not the post-parse body.
    if parsed.body.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    let policy = resolve_content_policy(repo, room.world_id).await;
    let content_segments = sanitize(&parsed.body, &policy);
    // `source` = raw author input for client edit-prefill, with a parsed /w
    // prefix STRIPPED — an unmodified resubmit of the prefill must not trip
    // handle_edit_message's AudienceLocked rejection (edit always rejects /w).
    let source = Some(if had_whisper {
        parsed.body.clone()
    } else {
        content.clone()
    });
    let doc = build_message_doc(
        room.world_id,
        ctx.user_id,
        channel,
        actor_owner,
        audience,
        parsed.kind,
        content_segments,
        source,
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
    .map_err(SendMessageError::Data)
}

/// Server-authoritative message edit: owner-or-GM only, re-runs the same
/// command-parse + sanitize pipeline `handle_send_message` uses, and rewrites
/// ONLY `content`/`kind`/`edited_at` on the stored `/system` body —
/// `channel`/`user_owner`/`actor_owner`/`audience`/`deleted_at` are copied
/// verbatim from the stored document, never re-derived from the edit request.
/// A `/w` (or any whisper-targeting content) in the edit is rejected as
/// `AudienceLocked` rather than silently retargeting the audience — the sole
/// place this function may reach `Room::publish` uses
/// `WriteOrigin::ServerMessageRevision`, the ONLY origin that re-opens the
/// `apply_intent` Update blanket-rejection for a stored `message` doc.
#[allow(clippy::too_many_arguments)]
pub async fn handle_edit_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    message_id: Uuid,
    content: String,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if content.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }

    let cur = repo
        .get_document(message_id)
        .await
        .map_err(SendMessageError::Data)?
        .ok_or(SendMessageError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return Err(SendMessageError::NotFound);
    }
    // Authorize: message owner OR a GM.
    let is_gm = ctx.world_role == WorldRole::Gm;
    if cur.owner != Some(ctx.user_id) && !is_gm {
        return Err(SendMessageError::Forbidden);
    }

    // A tombstoned message is, from the edit path's perspective, gone: reuse
    // NotFound rather than adding a new variant. Without this, an owner/GM
    // could resurrect `content` (re-indexed into FTS) on a doc whose
    // `deleted_at` marker stays set — simultaneously "deleted" and
    // content-bearing, defeating the soft-delete's content-clearing intent.
    let mut sys: MessageSystem = serde_json::from_value(cur.system.clone())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    if sys.deleted_at.is_some() {
        return Err(SendMessageError::NotFound);
    }

    let parsed = parse_command(&content);
    // Audience is frozen on edit — a /w in an edit is rejected, not applied.
    if parsed.whisper_to.is_some() {
        return Err(SendMessageError::AudienceLocked);
    }
    if parsed.body.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }

    let policy = resolve_content_policy(repo, room.world_id).await;
    let segments = sanitize(&parsed.body, &policy);

    // Build the revised system: new content + kind, edited_at=now; preserve
    // channel/user_owner/actor_owner/audience/deleted_at from the stored doc.
    sys.content = segments;
    sys.kind = parsed.kind;
    // Edit always rejects a /w (AudienceLocked, checked above), so the full
    // content is always the correct source here — no strip needed.
    sys.source = Some(content.clone());
    sys.edited_at = Some(now);
    let new_system = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            path: "/system".into(),
            old: cur.system,
            new: new_system,
        }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(SendMessageError::Data)
}

/// Server-authoritative message soft-delete: owner-or-GM only, a pure
/// tombstone — no command parsing or sanitization runs. Clears `content` and
/// sets `deleted_at`; `channel`/`user_owner`/`actor_owner`/`audience`/`kind`/
/// `edited_at` are left untouched. Like `handle_edit_message`, the write is
/// an `Operation::Update` on `/system` under `WriteOrigin::ServerMessageRevision`
/// (not a hard `Operation::Delete`) — the doc stays in the sequenced log, so
/// resync and per-recipient redaction both continue to apply to it unchanged.
///
/// Rate-limited like `handle_send_message`/`handle_edit_message`: the OCC
/// pre-image is re-read fresh from the current stored doc on every call
/// (always matches) and `deleted_at` is re-stamped with a fresh `now` each
/// time, so without a flood budget a single owner/GM could repeatedly
/// `DeleteMessage` the same message — each call consuming a real seq number,
/// broadcasting to every world member, and re-writing the FTS index — an
/// unbounded write/broadcast amplification from one cheap authenticated frame.
pub async fn handle_delete_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    message_id: Uuid,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }
    let cur = repo
        .get_document(message_id)
        .await
        .map_err(SendMessageError::Data)?
        .ok_or(SendMessageError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return Err(SendMessageError::NotFound);
    }
    // Authorize: message owner OR a GM (same rule as edit).
    let is_gm = ctx.world_role == WorldRole::Gm;
    if cur.owner != Some(ctx.user_id) && !is_gm {
        return Err(SendMessageError::Forbidden);
    }

    let mut sys: MessageSystem = serde_json::from_value(cur.system.clone())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    sys.content = Vec::new();
    // Clear alongside content — a retained source would leak deleted content
    // through the envelope (edit-prefill data is otherwise unredacted).
    sys.source = None;
    sys.deleted_at = Some(now);
    let new_system = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            path: "/system".into(),
            old: cur.system,
            new: new_system,
        }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(SendMessageError::Data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, Scope};
    use uuid::Uuid;

    #[test]
    fn html_segment_tagged_roundtrip() {
        let s = Segment::Html {
            sanitized_html: "<em>hi</em>".into(),
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["kind"], "html");
        assert_eq!(j["sanitized_html"], "<em>hi</em>");
        assert_eq!(s, serde_json::from_value(j).unwrap());
    }

    #[test]
    fn message_system_omits_absent_edit_delete_markers() {
        let sys = MessageSystem {
            channel: "all".into(),
            user_owner: Uuid::from_u128(1),
            actor_owner: None,
            kind: MessageKind::Normal,
            audience: Audience::Public,
            content: plain_text_content("hi"),
            source: None,
            edited_at: None,
            deleted_at: None,
        };
        let j = serde_json::to_value(&sys).unwrap();
        assert!(
            j.get("edited_at").is_none(),
            "None edited_at must not serialize"
        );
        assert!(
            j.get("deleted_at").is_none(),
            "None deleted_at must not serialize"
        );
        // Round-trips (a stored c-1 message with no markers deserializes unchanged).
        assert_eq!(sys, serde_json::from_value(j).unwrap());
    }

    #[test]
    fn build_message_doc_threads_kind() {
        let doc = build_message_doc(
            Uuid::from_u128(10),
            Uuid::from_u128(20),
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Emote,
            plain_text_content("waves"),
            None,
            1,
        );
        let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
        assert_eq!(sys.kind, MessageKind::Emote);
    }

    #[test]
    fn actor_owner_ref_tagged_roundtrip() {
        let a = ActorOwnerRef::Actor {
            actor_id: Uuid::from_u128(1),
        };
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j["kind"], "actor");
        assert_eq!(a, serde_json::from_value(j).unwrap());

        let t = ActorOwnerRef::TokenInstance {
            token_id: Uuid::from_u128(2),
        };
        let j = serde_json::to_value(&t).unwrap();
        assert_eq!(j["kind"], "token_instance");
        assert_eq!(t, serde_json::from_value(j).unwrap());
    }

    #[test]
    fn message_kind_defaults_normal_snake_case() {
        assert_eq!(MessageKind::default(), MessageKind::Normal);
        assert_eq!(
            serde_json::to_value(MessageKind::System).unwrap(),
            serde_json::json!("system")
        );
    }

    #[test]
    fn plain_text_produces_single_text_segment() {
        let segs = plain_text_content("hello <b>world</b>");
        assert_eq!(
            segs,
            vec![Segment::Text {
                text: "hello <b>world</b>".into()
            }]
        );
        // Producer stores raw text verbatim; markup is inert data, rendered as text (M11d).
        let j = serde_json::to_value(&segs[0]).unwrap();
        assert_eq!(j["kind"], "text");
        assert_eq!(j["text"], "hello <b>world</b>");
    }

    #[test]
    fn plain_text_empty_is_empty_segment() {
        assert_eq!(
            plain_text_content(""),
            vec![Segment::Text {
                text: String::new()
            }]
        );
    }

    #[test]
    fn build_message_doc_is_server_owned_message() {
        let world = Uuid::from_u128(10);
        let user = Uuid::from_u128(20);
        let doc = build_message_doc(
            world,
            user,
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            plain_text_content("hi"),
            None,
            1234,
        );
        assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
        assert_eq!(doc.owner, Some(user));
        assert_eq!(doc.scope, Scope::World { world_id: world });
        assert_eq!(doc.created_at, 1234);
        // Author gets the Owner floor so the create WRITE_FIELDS check passes;
        // default Observer so every world member can read it.
        assert_eq!(doc.permissions.default, DocRole::Observer);
        assert_eq!(doc.permissions.users.get(&user), Some(&DocRole::Owner));
        // Body round-trips back to a MessageSystem with server-set user_owner.
        let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
        assert_eq!(sys.user_owner, user);
        assert_eq!(sys.channel, "all");
        assert_eq!(sys.kind, MessageKind::Normal);
        assert_eq!(sys.audience, Audience::Public);
        assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
    }

    #[test]
    fn ops_target_message_detects_message_create_and_update() {
        let msg = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            vec![],
            None,
            0,
        );
        assert!(ops_target_message(&[Operation::Create {
            doc: msg.clone()
        }]));
        assert!(ops_target_message(&[Operation::Delete { doc: msg }]));

        let mut note = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            vec![],
            None,
            0,
        );
        note.doc_type = "note".into();
        assert!(!ops_target_message(&[Operation::Create { doc: note }]));
    }

    #[test]
    fn ops_target_message_detects_message_in_mixed_batch() {
        // A batch with one innocuous non-message op followed by a message
        // Create must still trip the guard: `.any()` must not short-circuit
        // on the first (non-matching) op.
        let mut note = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            vec![],
            None,
            0,
        );
        note.doc_type = "note".into();
        let msg = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            vec![],
            None,
            0,
        );
        assert!(ops_target_message(&[
            Operation::Create { doc: note },
            Operation::Create { doc: msg },
        ]));
    }

    #[test]
    fn audience_tagged_roundtrip_and_default() {
        let w = Audience::Whisper {
            recipients: vec![Uuid::from_u128(1)],
        };
        let j = serde_json::to_value(&w).unwrap();
        assert_eq!(j["kind"], "whisper");
        assert_eq!(w, serde_json::from_value(j).unwrap());
        assert_eq!(
            serde_json::to_value(Audience::GmOnly).unwrap()["kind"],
            "gm_only"
        );
        assert_eq!(Audience::default(), Audience::Public);
    }

    #[test]
    fn build_message_doc_public_matches_c1_shape() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            plain_text_content("hi"),
            None,
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::Observer);
        assert_eq!(doc.permissions.gm_role, None);
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    }

    #[test]
    fn build_message_doc_whisper_restricts_default_and_gm() {
        let owner = Uuid::from_u128(1);
        let recipient = Uuid::from_u128(2);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "whispers".into(),
            None,
            Audience::Whisper {
                recipients: vec![recipient],
            },
            MessageKind::Normal,
            plain_text_content("psst"),
            None,
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::None);
        assert_eq!(doc.permissions.gm_role, Some(DocRole::None));
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
        assert_eq!(
            doc.permissions.users.get(&recipient),
            Some(&DocRole::Observer)
        );
    }

    #[test]
    fn build_message_doc_whisper_self_recipient_does_not_downgrade_owner() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "whispers".into(),
            None,
            Audience::Whisper {
                recipients: vec![owner],
            },
            MessageKind::Normal,
            plain_text_content("note to self"),
            None,
            0,
        );
        assert_eq!(
            doc.permissions.users.get(&owner),
            Some(&DocRole::Owner),
            "a redundant self-recipient must never downgrade the owner to Observer"
        );
    }

    #[test]
    fn build_message_doc_gm_only_has_no_named_recipients() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "gm".into(),
            None,
            Audience::GmOnly,
            MessageKind::Normal,
            plain_text_content("only the GM sees this"),
            None,
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::None);
        assert_eq!(doc.permissions.gm_role, Some(DocRole::Observer));
        assert_eq!(
            doc.permissions.users.len(),
            1,
            "only the owner is individually listed — every GM sees it dynamically via gm_role"
        );
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    }

    #[tokio::test]
    async fn handle_send_message_publishes_and_broadcasts() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let (mut rx, _current) = room.subscribe();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "hello".into(),
            None,
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd.seq, 1);
        let got = rx.recv().await.unwrap();
        assert_eq!(got.event_seq(), Some(1));

        // Rate limit: exhaust the budget then expect RateLimited.
        let rate2 = PingRateLimiter::new();
        for _ in 0..2 {
            let _ = handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate2,
                "all".into(),
                "x".into(),
                None,
                Audience::Public,
                100,
                2,
            )
            .await;
        }
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate2,
            "all".into(),
            "x".into(),
            None,
            Audience::Public,
            100,
            2,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::RateLimited)));

        // Empty + too-long rejected before any publish.
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "all".into(),
                "".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::Empty)
        ));
        let long = "a".repeat(MAX_MESSAGE_CHARS + 1);
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "all".into(),
                long,
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::TooLong)
        ));

        // Empty/over-long channel rejected before any publish; seq unchanged.
        let seq_before = room.subscribe().1;
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "".into(),
                "hi".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::Empty)
        ));
        let long_channel = "c".repeat(MAX_CHANNEL_CHARS + 1);
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                long_channel,
                "hi".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::TooLong)
        ));
        assert_eq!(
            room.subscribe().1,
            seq_before,
            "rejected channel must not publish"
        );
    }

    #[tokio::test]
    async fn handle_send_message_rejects_unknown_whisper_recipient() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // A uuid that belongs to no user at all, let alone this world.
        let foreign = Uuid::from_u128(99_999);
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper {
                recipients: vec![foreign],
            },
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::UnknownRecipient)));

        // Nothing was persisted — the seq was never consumed.
        assert!(repo.events_since(w.id, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_send_message_accepts_a_whisper_to_a_real_member() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let recipient = repo
            .create_user("re", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, recipient, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper {
                recipients: vec![recipient],
            },
            100,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd.seq, 1);
    }

    #[tokio::test]
    async fn handle_send_message_rejects_oversized_whisper_recipient_list() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // One over the cap: none of these uuids belong to this world, so if
        // the cap check ran AFTER the per-recipient member_role loop this
        // would instead fail with UnknownRecipient — proving the cap check
        // runs FIRST, before any member_role query.
        let recipients: Vec<Uuid> = (0..(MAX_WHISPER_RECIPIENTS as u128 + 1))
            .map(Uuid::from_u128)
            .collect();
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper { recipients },
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::TooLong)));
        assert!(
            repo.events_since(w.id, 0).await.unwrap().is_empty(),
            "an oversized whisper must persist nothing"
        );
    }

    #[tokio::test]
    async fn handle_send_message_accepts_whisper_at_exactly_the_recipient_cap() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // Exactly at the cap, all recipients are the sender themself (a
        // no-op member_role lookup that always succeeds) — this test proves
        // the boundary is accepted, not just that over-the-limit is rejected.
        let recipients: Vec<Uuid> = std::iter::repeat_n(player, MAX_WHISPER_RECIPIENTS).collect();
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper { recipients },
            100,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd.seq, 1);
    }

    /// A message doc built via `build_message_doc` and committed via
    /// `apply_intent` under the posting Player's own ctx (the same write
    /// `handle_send_message` performs) is found by ANOTHER world member's
    /// `repo.search` — the message rides the existing search index with no
    /// message-specific indexing code, and its body text surfaces in the
    /// snippet.
    #[tokio::test]
    async fn source_stores_raw_input_for_plain_and_command_messages() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let alice = repo
            .create_user("alice", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, alice, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // Plain message: source == the full content.
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "hello".into(),
            None,
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc,
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
        assert_eq!(sys.source, Some("hello".into()));

        // Command message: source keeps the command prefix (re-parses identically).
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "/me waves".into(),
            None,
            Audience::Public,
            101,
            30,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc,
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
        assert_eq!(sys.source, Some("/me waves".into()));

        // Whisper via content /w: source has the /w prefix STRIPPED.
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "/w @alice hi".into(),
            None,
            Audience::Public,
            102,
            30,
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc,
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
        assert_eq!(sys.source, Some("hi".into()));
    }

    #[tokio::test]
    async fn edit_replaces_source_and_delete_clears_it() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "hello".into(),
            None,
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        let message_id = match &cmd.ops[0] {
            Operation::Create { doc } => doc.id,
            other => panic!("expected Create, got {other:?}"),
        };

        handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            message_id,
            "goodbye".into(),
            101,
            30,
        )
        .await
        .unwrap();
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys: MessageSystem = serde_json::from_value(stored.system.clone()).unwrap();
        assert_eq!(sys.source, Some("goodbye".into()));

        handle_delete_message(&room, &repo, &ctx, &rate, message_id, 102, 30)
            .await
            .unwrap();
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys: MessageSystem = serde_json::from_value(stored.system).unwrap();
        assert_eq!(sys.source, None, "delete tombstone must clear source");
        assert!(
            sys.content.is_empty(),
            "delete tombstone must clear content"
        );
    }

    #[test]
    fn stored_pre_source_message_still_deserializes() {
        // A stored c-3 (pre-`source`) MessageSystem JSON has no `source` key at all.
        let j = serde_json::json!({
            "channel": "all",
            "user_owner": Uuid::from_u128(1),
            "kind": "normal",
            "audience": { "kind": "public" },
            "content": [],
        });
        let sys: MessageSystem = serde_json::from_value(j).unwrap();
        assert_eq!(sys.source, None);
    }

    #[tokio::test]
    async fn posted_message_is_searchable_by_members() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = r
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let other = r
            .create_user("ot", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        r.add_member(w.id, player, WorldRole::Player).await.unwrap();
        r.add_member(w.id, other, WorldRole::Player).await.unwrap();
        let pl_ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let ot_ctx = PermissionContext {
            user_id: other,
            world_role: WorldRole::Player,
        };

        let doc = build_message_doc(
            w.id,
            player,
            "all".into(),
            None,
            Audience::Public,
            MessageKind::Normal,
            plain_text_content("banshee wail"),
            None,
            1,
        );
        r.apply_intent(
            &pl_ctx,
            w.id,
            vec![Operation::Create { doc }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let page = r.search(&ot_ctx, w.id, "banshee", 10, None).await.unwrap();
        assert_eq!(page.hits.len(), 1, "another member finds the message");
        assert!(page.hits[0].snippet.to_lowercase().contains("banshee"));
    }
}
