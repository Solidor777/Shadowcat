//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with a typed, ingress-validated
//! `engine` body (this module's `MessageEngine`, M13-0: re-rooted from the
//! opaque `system` band `MessageSystem` used pre-M13-0); `system` stays
//! reserved-empty (`{}`) for message docs. Authored and revised ONLY by the
//! server — never built or mutated by a client directly. A `message` doc_type reaches
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
mod link_preview;
mod preview_cache;
mod rolls;
mod sanitize;
mod settings;
mod shortcodes;
pub use commands::{parse_command, ParsedCommand};
pub use link_preview::{
    build_client as build_link_preview_client, enrich as enrich_link_previews, fetch_preview,
    LinkPreview, PreviewError, MAX_PREVIEWS_PER_MESSAGE,
};
pub use preview_cache::{
    LinkPreviewCache, PreviewRateLimiter, MAX_CACHE_ENTRIES, NEGATIVE_TTL, POSITIVE_TTL,
    PREVIEW_FETCH_PER_MIN,
};
pub use sanitize::sanitize;
pub use settings::{
    resolve_content_policy, resolve_dice_context, ChatContentPolicy, CHAT_SETTINGS_DOC_TYPE,
    DICE_SETTINGS_DOC_TYPE,
};

use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::dice::RollOutcome;
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
/// `SendMessage` frame and stored in `MessageEngine`. No ID newtypes exist —
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
/// `MessageEngine`; drives the document's `PermissionSet` in
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
    /// A completed roll: the formula plus its full deterministic outcome.
    /// `outcome` embeds the evaluated `RollOutcome` (records included — the
    /// natural faces make the roll reproducible/auditable from the stored
    /// segment alone). The `RollSpec`/`RawRoll` are deliberately NOT stored —
    /// recalculate-from-chat is out of scope pre-release, so there is no
    /// data-continuity promise to a future recalc feature. Produced only by
    /// `chat::rolls::execute_roll`, called from `handle_send_message`'s roll
    /// stage; never produced on edit (rolls are immutable, see
    /// `handle_edit_message`).
    RollEmbed {
        formula: String,
        outcome: RollOutcome,
    },
    /// An unexecuted, parse-and-cap-validated formula the card renders as a
    /// button; clicking it sends a fresh `/roll <formula>` `SendMessage`
    /// (a new, independently-attributed roll). `label` is plain data (never
    /// rendered as markup).
    RollButton {
        formula: String,
        label: Option<String>,
    },
    /// A server-fetched, SSRF-guarded preview of a link in the message.
    /// Rendered by the client from STORED data only — the client never
    /// fetches `url` or any remote resource (that would leak the viewer's
    /// IP). Appended at the END of `content` by `link_preview::enrich`,
    /// after every other segment the send/edit pipeline produced (`enrich`
    /// never reorders or removes an existing segment).
    LinkPreview {
        url: String,
        title: String,
        description: String,
    },
    // Reserved, produced later: DocLink (M11d).
}

/// The c-1 producer: wrap raw input as a single literal-text segment. Rich
/// producers (markdown/HTML) are added in c-3, feeding this same content model.
pub fn plain_text_content(raw: &str) -> Vec<Segment> {
    vec![Segment::Text {
        text: raw.to_string(),
    }]
}

/// The message document's `engine` body (M13-0: re-rooted from `system`).
/// Opaque on the WIRE (no ts-rs — the client declares its own Zod mirror,
/// M11d's `chat-docs.ts`), but ingress-validated server-side same as every
/// other engine-defined doc_type: `deny_unknown_fields` closes the gap a
/// pre-M13-0 `MessageSystem` left open (an unknown key on this body used to
/// pass through the opaque `system` band unrejected).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageEngine {
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
    /// A WHISPER's `source` is stored post-`/w`-strip (the literal body, not
    /// the raw "/w @user ..." prefix) precisely so an unmodified prefill
    /// resubmit round-trips: `handle_edit_message` skips command parsing
    /// entirely for a whisper (mirroring `handle_send_message`'s own
    /// literal-body treatment of a whisper's content), so nested command-like
    /// text in a whisper body is never parsed on send OR on edit.
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
    let engine = MessageEngine {
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
    let engine_json = serde_json::to_value(&engine).expect("MessageEngine serializes");
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        // Messages have no envelope display name — `name` is `None` for the
        // entire doc lifetime (never set on edit/delete either).
        name: None,
        source: None,
        base: None,
        owner: Some(user),
        permissions: PermissionSet {
            default,
            users,
            gm_role,
            ..Default::default()
        },
        embedded: BTreeMap::new(),
        parent_id: None,
        // `message` is engine-defined (`data::engine::is_engine_doc_type`):
        // the real content lives in `engine` only; `system` stays
        // reserved-empty (`{}`) for message docs — there is no game-system
        // data on a chat message.
        engine: Some(engine_json),
        system: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// Author a `MessageKind::System` error notice for a failed roll attempt:
/// whispered to the sender only (same channel), owned by the sender (so they
/// may delete it), one `Text` segment with the error's player-presentable
/// `Display` text. This is `MessageKind::System`'s first real producer —
/// deliberately NOT `parse_command` (which can never emit `System`, proven by
/// its own exhaustive test); a roll failure is authored directly here instead.
fn build_roll_error_notice(
    world_id: Uuid,
    sender: Uuid,
    channel: String,
    err: &rolls::RollError,
    now: i64,
) -> Document {
    build_message_doc(
        world_id,
        sender,
        channel,
        None,
        Audience::Whisper {
            recipients: vec![sender],
        },
        MessageKind::System,
        vec![Segment::Text {
            text: err.to_string(),
        }],
        None,
        now,
    )
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
    /// A roll formula failed to parse, exceeded a wire-boundary cap, or a
    /// message body's inline-roll scan failed. Never returned to the caller
    /// as a hard error — `handle_send_message` catches this and authors a
    /// `MessageKind::System` notice instead (see `build_roll_error_notice`);
    /// kept as a variant for completeness/testability of the mapping.
    Roll(rolls::RollError),
    /// A roll's outcome is immutable once sent: editing a message whose
    /// STORED `kind == Roll`, or editing content that itself parses to
    /// `kind == Roll`, are both rejected outright (no re-rolling by edit).
    RollImmutable,
    /// The requested `actor_owner` cannot be attributed by this sender: the
    /// referenced actor doc does not exist, is not an `actor` doc_type, or
    /// is owned by someone else and the sender is not a GM; or the ref is a
    /// `TokenInstance` (rejected fail-closed pending speak-as-token, design
    /// doc §8).
    ActorNotSpeakable,
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
    preview_client: &reqwest::Client,
    preview_cache: &LinkPreviewCache,
    preview_rate: &PreviewRateLimiter,
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
    // Attribution ownership gate (spec §8): `actor_owner` is client-supplied
    // and otherwise stored verbatim — without this check any world member
    // could attribute a message to ANY actor doc, spoofing its display name
    // to every recipient who can read the message. Fail-closed, whole-send:
    // an invalid ref rejects BEFORE any content parsing/sanitization/roll
    // execution runs, exactly like the whisper-recipient validation below.
    // `handle_edit_message` copies `actor_owner` verbatim from the STORED
    // doc, never from the edit request, so this ingest-time gate is the
    // ONLY place attribution is ever chosen — no separate edit-time check
    // is needed.
    if let Some(owner_ref) = &actor_owner {
        match owner_ref {
            ActorOwnerRef::Actor { actor_id } => {
                let actor_doc = repo
                    .get_document(*actor_id)
                    .await
                    .map_err(SendMessageError::Data)?;
                let is_gm = ctx.world_role == WorldRole::Gm;
                let allowed = match &actor_doc {
                    // GM may attribute as any existing actor doc; a Player
                    // only as an actor doc they own.
                    Some(d) if d.doc_type == "actor" => is_gm || d.owner == Some(ctx.user_id),
                    _ => false,
                };
                if !allowed {
                    return Err(SendMessageError::ActorNotSpeakable);
                }
            }
            // No first-party producer resolves a TokenInstance ref into a
            // display identity at send time (speak-as-token is an
            // explicitly deferred M11d follow-up, design doc §8) — reject
            // fail-closed rather than store an unvalidated ref that a
            // future consumer might trust.
            ActorOwnerRef::TokenInstance { .. } => {
                return Err(SendMessageError::ActorNotSpeakable);
            }
        }
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
    // Roll stage: `parsed.kind`/an inline `[[...]]`/`[[roll:...]]` span in the
    // body determines whether (part of) the body executes as dice notation
    // before sanitize runs. INVARIANT: exactly ONE message is authored per
    // send attempt — a roll/scan failure authors a `MessageKind::System`
    // notice INSTEAD of the intended message (never in addition), so the
    // flood budget already consumed above stays 1:1 with the attempt.
    // `handle_edit_message` never reaches this stage: a roll's outcome is
    // immutable once sent, and an edit's `[[...]]` spans stay literal text.
    //
    // Resolved ONCE here (not per-branch below) so the link-preview enrich
    // stage below can reuse the same resolution for `previews_enabled()`
    // without a second query. The `kind == Roll` branch below never reads
    // `policy` (a roll's body is executed, not sanitized) — it is resolved
    // and ignored on that path; the hoist is for the shared Normal/Emote
    // sanitize call and the enrich gate, both of which run regardless of
    // whether this attempt turns out to be a roll.
    let policy = resolve_content_policy(repo, room.world_id).await;
    let mut content_segments = if parsed.kind == MessageKind::Roll {
        let dice_ctx = resolve_dice_context(repo, room.world_id).await;
        match rolls::execute_roll(&parsed.body, dice_ctx) {
            Ok((formula, outcome)) => vec![Segment::RollEmbed { formula, outcome }],
            Err(e) => {
                let notice = build_roll_error_notice(room.world_id, ctx.user_id, channel, &e, now);
                return room
                    .publish(
                        repo,
                        ctx,
                        vec![Operation::Create { doc: notice }],
                        now,
                        WriteOrigin::Client,
                    )
                    .await
                    .map_err(SendMessageError::Data);
            }
        }
    } else {
        // Normal/Emote: scan for inline rolls/buttons. The all-Text case is
        // the byte-identical fast path over the whole body (unchanged from
        // before this checkpoint); a mixed body sanitizes each Text chunk
        // independently and interleaves roll segments in scan order.
        let chunks = match rolls::scan_body(&parsed.body) {
            Ok(c) => c,
            Err(e) => {
                let notice = build_roll_error_notice(room.world_id, ctx.user_id, channel, &e, now);
                return room
                    .publish(
                        repo,
                        ctx,
                        vec![Operation::Create { doc: notice }],
                        now,
                        WriteOrigin::Client,
                    )
                    .await
                    .map_err(SendMessageError::Data);
            }
        };
        if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
            sanitize(&parsed.body, &policy)
        } else {
            // Ambient dice context is resolved at most once, lazily, only when
            // a roll/button chunk actually appears in this body.
            let mut dice_ctx: Option<crate::dice::ParseContext> = None;
            let mut segments = Vec::with_capacity(chunks.len());
            let mut roll_err = None;
            for chunk in chunks {
                match chunk {
                    rolls::BodyChunk::Text(t) => segments.extend(sanitize(t, &policy)),
                    rolls::BodyChunk::Inline(formula) => {
                        if dice_ctx.is_none() {
                            dice_ctx = Some(resolve_dice_context(repo, room.world_id).await);
                        }
                        match rolls::execute_roll(formula, dice_ctx.unwrap()) {
                            Ok((formula, outcome)) => {
                                segments.push(Segment::RollEmbed { formula, outcome })
                            }
                            Err(e) => {
                                roll_err = Some(e);
                                break;
                            }
                        }
                    }
                    rolls::BodyChunk::Button { formula, label } => {
                        if dice_ctx.is_none() {
                            dice_ctx = Some(resolve_dice_context(repo, room.world_id).await);
                        }
                        // Stored/validated formula is trimmed — the `roll:`/`|`
                        // split leaves incidental whitespace (e.g.
                        // "[[roll: 1d20|Attack]]") that must not survive into
                        // the button's stored formula or the click-to-send text.
                        let formula = formula.trim();
                        match rolls::validate_formula(formula, dice_ctx.unwrap()) {
                            Ok(()) => segments.push(Segment::RollButton {
                                formula: formula.to_string(),
                                label: label.map(|s| s.to_string()),
                            }),
                            Err(e) => {
                                roll_err = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(e) = roll_err {
                let notice = build_roll_error_notice(room.world_id, ctx.user_id, channel, &e, now);
                return room
                    .publish(
                        repo,
                        ctx,
                        vec![Operation::Create { doc: notice }],
                        now,
                        WriteOrigin::Client,
                    )
                    .await
                    .map_err(SendMessageError::Data);
            }
            segments
        }
    };
    // Link-preview enrich stage: only for hyperlink-carrying, non-Roll bodies.
    // The `kind != Roll` guard is EXPLICIT (design doc §3), not incidental: a
    // successful roll falls through here with `content_segments == [RollEmbed]`
    // (only the roll-EXECUTION-FAILURE arm returns early), so without this
    // guard a `/roll` on a preview-enabled world would enter `enrich` — a no-op
    // today only because `enrich` scans `Segment::Html` runs (none in a
    // RollEmbed), but a latent path to attaching outbound-fetched previews to a
    // roll message if that ever changes. Synchronous, before publish (§3) — no
    // spawned task, no post-publish revision, no message-deleted-mid-fetch race.
    if parsed.kind != MessageKind::Roll && policy.previews_enabled() {
        link_preview::enrich(
            &mut content_segments,
            preview_client,
            preview_cache,
            preview_rate,
            ctx.user_id,
            now,
            std::time::Instant::now(),
        )
        .await;
    }
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

/// Server-authoritative message edit: owner-or-GM only, and rewrites ONLY
/// `content`/`kind`/`edited_at` on the stored `/engine` body (M13-0:
/// re-rooted from `/system`) — `channel`/`user_owner`/`actor_owner`/
/// `audience`/`deleted_at` are copied verbatim from the stored document,
/// never re-derived from the edit request.
///
/// For a NON-WHISPER message, this re-runs the same command-parse + sanitize
/// pipeline `handle_send_message` uses; a `/w` (or any whisper-targeting
/// content) in the edit is rejected as `AudienceLocked` rather than silently
/// retargeting the audience.
///
/// For a WHISPER message, command parsing is skipped entirely — the edit
/// content is treated as the literal body (mirroring `handle_send_message`'s
/// own literal-body treatment of whisper content) and `kind` is left as
/// stored. Without this, re-running `parse_command` on an unmodified prefill
/// resubmit of a stored whisper body could silently mutate `kind` (e.g. a
/// stored "/me waves" body reparsing to `Emote`) or spuriously trip
/// `AudienceLocked` on a literal "/w ..." body — the exact idempotency
/// failure the `source`-strip mechanism exists to prevent, one token deeper.
/// `AudienceLocked` therefore only ever fires for a non-whisper message; a
/// whisper's literal body may legitimately contain "/w ..." text.
///
/// The sole place this function may reach `Room::publish` uses
/// `WriteOrigin::ServerMessageRevision`, the ONLY origin that re-opens the
/// `apply_intent` Update blanket-rejection for a stored `message` doc.
#[allow(clippy::too_many_arguments)]
pub async fn handle_edit_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    preview_client: &reqwest::Client,
    preview_cache: &LinkPreviewCache,
    preview_rate: &PreviewRateLimiter,
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
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap_or_default())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    if sys.deleted_at.is_some() {
        return Err(SendMessageError::NotFound);
    }
    // Roll immutability (anti-cheat): a roll's outcome is fixed at send time.
    // This check is UNCONDITIONAL (not gated on audience) because
    // `kind == Roll` + `audience == Whisper` IS reachable: `handle_send_message`
    // always runs `parse_command` on the raw content regardless of which
    // front-door set the audience (the c-2 frame field or a content `/w`) — a
    // frame `SendMessage{content: "/roll 2d6", audience: Whisper{..}}` parses
    // to `kind: Roll` with no `/w` in the content, so the frame's `Whisper`
    // audience is used verbatim alongside `kind: Roll`. Without this
    // unconditional placement, a whispered roll's audit record could be
    // edited away.
    //
    // A message carrying ANY roll segment is edit-immutable, not just one
    // whose STORED `kind` is `Roll`: a Normal/Emote message's body can embed
    // an inline `[[...]]` roll or a `[[roll:...]]` button mid-text (e.g.
    // "attack! [[1d20]] done"), producing `Segment::RollEmbed`/
    // `Segment::RollButton` entries inside an otherwise-Normal message's
    // `content`. Editing that message's text would otherwise silently erase
    // the executed roll's audit record even though the top-level `kind`
    // check never fires. Delete remains available for any message.
    let has_roll_segment = sys
        .content
        .iter()
        .any(|seg| matches!(seg, Segment::RollEmbed { .. } | Segment::RollButton { .. }));
    if sys.kind == MessageKind::Roll || has_roll_segment {
        return Err(SendMessageError::RollImmutable);
    }

    // A stored WHISPER's literal body may legitimately contain "/w ..." or any
    // other leading command token — `handle_send_message` never parses a
    // whisper's body as a command either (see `source`'s doc comment: a send's
    // `source` for a whisper is the POST-parse body, so an unmodified prefill
    // resubmit is exactly this literal text). Re-parsing it here would silently
    // mutate `kind` (e.g. a stored "/me waves" whisper body re-parsing to
    // `Emote`) or re-trip `AudienceLocked` on a stored "/w ..." literal — the
    // exact failure the `source`/prefill mechanism exists to prevent, one
    // token deeper. Skip parsing entirely for a whisper edit and keep the
    // stored `kind`; the `AudienceLocked` rejection below applies only to
    // non-whisper messages, where a literal "/w ..." body is unexpected.
    let is_whisper = matches!(sys.audience, Audience::Whisper { .. });
    let (kind, body) = if is_whisper {
        (sys.kind, content.clone())
    } else {
        let parsed = parse_command(&content);
        // Audience is frozen on edit — a /w in an edit is rejected, not applied.
        if parsed.whisper_to.is_some() {
            return Err(SendMessageError::AudienceLocked);
        }
        // Roll immutability: editing content INTO a roll (e.g. a plain message
        // edited to "/roll 1d6") is rejected the same as editing a message
        // that already IS one — no editing-in-to a roll either. Edits also
        // never call `scan_body`: an edit's `[[...]]` spans stay literal text
        // through the ordinary sanitize path below (never re-executed).
        if parsed.kind == MessageKind::Roll {
            return Err(SendMessageError::RollImmutable);
        }
        (parsed.kind, parsed.body)
    };
    if body.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }

    let policy = resolve_content_policy(repo, room.world_id).await;
    let mut segments = sanitize(&body, &policy);
    // A preview is derived, not authored — re-derive on every edit so the
    // card always reflects the CURRENT edited content (never a stale link
    // preview from before the edit). The roll-immutability checks above
    // already guarantee `kind != Roll` here.
    if policy.previews_enabled() {
        link_preview::enrich(
            &mut segments,
            preview_client,
            preview_cache,
            preview_rate,
            ctx.user_id,
            now,
            std::time::Instant::now(),
        )
        .await;
    }

    // Build the revised engine band: new content + kind, edited_at=now;
    // preserve channel/user_owner/actor_owner/audience/deleted_at from the
    // stored doc.
    sys.content = segments;
    sys.kind = kind;
    // Whisper edits skip parsing (body IS the literal content above); non-
    // whisper edits always reject a /w (AudienceLocked, checked above) — either
    // way the full raw content is the correct source here, no strip needed.
    sys.source = Some(content.clone());
    sys.edited_at = Some(now);
    let new_engine = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
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
/// an `Operation::Update` on `/engine` under `WriteOrigin::ServerMessageRevision`
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

    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap_or_default())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    sys.content = Vec::new();
    // Clear alongside content — a retained source would leak deleted content
    // through the envelope (edit-prefill data is otherwise unredacted).
    sys.source = None;
    sys.deleted_at = Some(now);
    let new_engine = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
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
        let sys = MessageEngine {
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
        let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
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
        // Body round-trips back to a MessageEngine with server-set user_owner.
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
                &super::link_preview::build_client_allow_loopback(),
                &LinkPreviewCache::new(),
                &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
                &super::link_preview::build_client_allow_loopback(),
                &LinkPreviewCache::new(),
                &PreviewRateLimiter::new(),
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
                &super::link_preview::build_client_allow_loopback(),
                &LinkPreviewCache::new(),
                &PreviewRateLimiter::new(),
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
                &super::link_preview::build_client_allow_loopback(),
                &LinkPreviewCache::new(),
                &PreviewRateLimiter::new(),
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
                &super::link_preview::build_client_allow_loopback(),
                &LinkPreviewCache::new(),
                &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.source, Some("hello".into()));

        // Command message: source keeps the command prefix (re-parses identically).
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.source, Some("/me waves".into()));

        // Whisper via content /w: source has the /w prefix STRIPPED.
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id,
            "goodbye".into(),
            101,
            30,
        )
        .await
        .unwrap();
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.source, Some("goodbye".into()));

        handle_delete_message(&room, &repo, &ctx, &rate, message_id, 102, 30)
            .await
            .unwrap();
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
        assert_eq!(sys.source, None, "delete tombstone must clear source");
        assert!(
            sys.content.is_empty(),
            "delete tombstone must clear content"
        );
    }

    #[tokio::test]
    async fn whisper_edit_prefill_resubmit_is_idempotent() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let sender = repo
            .create_user("sender", None, ServerRole::User, 0)
            .await
            .unwrap();
        let alice = repo
            .create_user("alice", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, sender, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, alice, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: sender,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // Send "/w @alice /me waves": a nested command inside a whisper body is
        // NOT parsed — stored kind is Normal, content/source are the literal
        // post-/w-strip body "/me waves".
        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "/w @alice /me waves".into(),
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
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.kind, MessageKind::Normal);
        assert_eq!(sys.source, Some("/me waves".into()));
        assert!(matches!(sys.audience, Audience::Whisper { .. }));

        // Edit-resubmit of the UNMODIFIED prefill ("/me waves", the stored
        // source): kind/content/source must round-trip unchanged, not reparse
        // into MessageKind::Emote.
        handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id,
            "/me waves".into(),
            101,
            30,
        )
        .await
        .unwrap();
        let stored = repo.get_document(message_id).await.unwrap().unwrap();
        let sys2: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
        assert_eq!(
            sys2.kind,
            MessageKind::Normal,
            "must not reparse into Emote"
        );
        assert_eq!(sys2.source, Some("/me waves".into()));
        assert_eq!(sys2.content, sys.content);

        // A second whisper, sent as "/w @alice hi": stored source is the
        // post-strip literal "hi". Edit-resubmitting "hi" (which itself looks
        // like an ordinary /w-free body) must not spuriously trip
        // AudienceLocked — the whole point of skipping command parsing on a
        // whisper edit — and a resubmit of a literal "/w ..." body must also
        // survive without AudienceLocked (only a non-whisper message rejects a
        // literal /w-shaped edit body).
        let cmd2 = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "/w @alice hi".into(),
            None,
            Audience::Public,
            102,
            30,
        )
        .await
        .unwrap();
        let message_id2 = match &cmd2.ops[0] {
            Operation::Create { doc } => doc.id,
            other => panic!("expected Create, got {other:?}"),
        };
        let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
        let sys2_pre: MessageEngine =
            serde_json::from_value(stored2.engine.clone().unwrap()).unwrap();
        assert_eq!(sys2_pre.source, Some("hi".into()));

        // Edit-resubmit of a whisper's stored body that itself reads as a /w
        // command must NOT be rejected AudienceLocked — command parsing is
        // skipped entirely on a whisper edit.
        handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id2,
            "/w @bob hi".into(),
            103,
            30,
        )
        .await
        .expect("a whisper edit must never reject a literal /w-shaped body");
        let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
        let sys2_post: MessageEngine = serde_json::from_value(stored2.engine.unwrap()).unwrap();
        assert_eq!(sys2_post.kind, MessageKind::Normal);
        assert_eq!(sys2_post.source, Some("/w @bob hi".into()));
        // Audience must remain the ORIGINAL whisper's recipients — frozen, not
        // retargeted to @bob, despite the literal body reading as a /w command.
        assert!(
            matches!(sys2_post.audience, Audience::Whisper { ref recipients } if recipients == &vec![alice])
        );

        // Editing a PUBLIC (non-whisper) message with /w-shaped content still
        // rejects AudienceLocked — the fast path applies ONLY to whisper
        // messages, not to every message.
        let cmd3 = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "hello".into(),
            None,
            Audience::Public,
            104,
            30,
        )
        .await
        .unwrap();
        let public_id = match &cmd3.ops[0] {
            Operation::Create { doc } => doc.id,
            other => panic!("expected Create, got {other:?}"),
        };
        let err = handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            public_id,
            "/w @alice hi".into(),
            105,
            30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SendMessageError::AudienceLocked));

        // An ORDINARY whisper edit (genuinely different content) still
        // sanitizes and updates content/source.
        handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id2,
            "bye now".into(),
            106,
            30,
        )
        .await
        .unwrap();
        let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
        let sys2_final: MessageEngine = serde_json::from_value(stored2.engine.unwrap()).unwrap();
        assert_eq!(sys2_final.source, Some("bye now".into()));
        match &sys2_final.content[..] {
            [Segment::Text { text }] => assert_eq!(text, "bye now"),
            other => panic!("expected a single Text segment, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn editing_a_normal_message_with_an_inline_roll_segment_is_immutable() {
        // FIX 2: a Normal message whose body embeds an inline roll ("attack!
        // [[1d20]] done") stores kind: Normal (never Roll) but its content
        // still carries a Segment::RollEmbed mid-text. Editing must be
        // rejected the same as editing a top-level `/roll` message would be
        // -- otherwise the roll's audit record could be erased by editing
        // around it.
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "attack! [[1d20]] done".into(),
            None,
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        let (message_id, doc) = match &cmd.ops[0] {
            Operation::Create { doc } => (doc.id, doc),
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.kind, MessageKind::Normal);
        assert!(
            sys.content
                .iter()
                .any(|s| matches!(s, Segment::RollEmbed { .. })),
            "expected an inline RollEmbed segment, got {:?}",
            sys.content
        );

        let err = handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id,
            "attack! done, no roll".into(),
            101,
            30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SendMessageError::RollImmutable));

        // A plain Normal message (no roll segment) still edits fine.
        let cmd2 = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "hello there".into(),
            None,
            Audience::Public,
            102,
            30,
        )
        .await
        .unwrap();
        let plain_id = match &cmd2.ops[0] {
            Operation::Create { doc } => doc.id,
            other => panic!("expected Create, got {other:?}"),
        };
        handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            plain_id,
            "hello again".into(),
            103,
            30,
        )
        .await
        .expect("a plain Normal message must still edit fine");
    }

    #[tokio::test]
    async fn whisper_roll_via_frame_audience_is_edit_immutable() {
        // FIX 3: kind == Roll + audience == Whisper IS reachable via the c-2
        // frame's `audience` field (content has no /w, so parse_command never
        // sets whisper_to, and the frame's Whisper audience is used as-is
        // alongside kind: Roll). The unconditional kind == Roll check must
        // still block editing it.
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let sender = repo
            .create_user("sender", None, ServerRole::User, 0)
            .await
            .unwrap();
        let alice = repo
            .create_user("alice", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, sender, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, alice, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: sender,
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "/roll 2d6".into(),
            None,
            Audience::Whisper {
                recipients: vec![alice],
            },
            100,
            30,
        )
        .await
        .unwrap();
        let (message_id, doc) = match &cmd.ops[0] {
            Operation::Create { doc } => (doc.id, doc),
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        assert_eq!(
            sys.kind,
            MessageKind::Roll,
            "expected reachable kind: Roll + audience: Whisper combination"
        );
        assert!(matches!(sys.audience, Audience::Whisper { .. }));

        let err = handle_edit_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            message_id,
            "2d6 edited".into(),
            101,
            30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SendMessageError::RollImmutable));
    }

    #[test]
    fn stored_pre_source_message_still_deserializes() {
        // A stored c-3 (pre-`source`) MessageEngine JSON has no `source` key at all.
        let j = serde_json::json!({
            "channel": "all",
            "user_owner": Uuid::from_u128(1),
            "kind": "normal",
            "audience": { "kind": "public" },
            "content": [],
        });
        let sys: MessageEngine = serde_json::from_value(j).unwrap();
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

    /// A minimal actor doc for the attribution-ownership gate tests, seeded
    /// directly via `apply_command` (bypasses permission checks — these
    /// tests exercise `handle_send_message`'s attribution gate, not the
    /// actor-create authorization path).
    fn seed_actor_doc(id: Uuid, world: Uuid, owner: Option<Uuid>) -> Document {
        Document {
            id,
            scope: Scope::World { world_id: world },
            doc_type: "actor".into(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner,
            permissions: crate::data::document::PermissionSet::default(),
            embedded: Default::default(),
            parent_id: None,
            engine: None,
            system: serde_json::json!({ "name": "Goblin" }),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn send_message_allows_player_attributing_own_actor() {
        use crate::auth::role::ServerRole;
        use crate::data::command::UnsequencedCommand;
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
        let actor_id = Uuid::new_v4();
        repo.apply_command(UnsequencedCommand {
            world_id: w.id,
            author: player,
            ts: 0,
            ops: vec![Operation::Create {
                doc: seed_actor_doc(actor_id, w.id, Some(player)),
            }],
        })
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
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::Actor { actor_id }),
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
        let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
        assert_eq!(sys.actor_owner, Some(ActorOwnerRef::Actor { actor_id }));
    }

    #[tokio::test]
    async fn send_message_rejects_player_attributing_another_users_actor() {
        use crate::auth::role::ServerRole;
        use crate::data::command::UnsequencedCommand;
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
        let other = repo
            .create_user("ot", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, other, WorldRole::Player)
            .await
            .unwrap();
        let actor_id = Uuid::new_v4();
        repo.apply_command(UnsequencedCommand {
            world_id: w.id,
            author: other,
            ts: 0,
            ops: vec![Operation::Create {
                doc: seed_actor_doc(actor_id, w.id, Some(other)),
            }],
        })
        .await
        .unwrap();

        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // The seeded actor doc's own Create already consumed one seq;
        // capture it so the assertion below proves the REJECTED send
        // itself persisted nothing, not merely that the log is empty.
        let seq_before = repo.events_since(w.id, 0).await.unwrap().len();
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::Actor { actor_id }),
            Audience::Public,
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
        assert_eq!(
            repo.events_since(w.id, 0).await.unwrap().len(),
            seq_before,
            "spoofed attribution must persist nothing"
        );
    }

    #[tokio::test]
    async fn send_message_rejects_attributing_a_nonexistent_actor() {
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

        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::Actor {
                actor_id: Uuid::new_v4(),
            }),
            Audience::Public,
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
    }

    #[tokio::test]
    async fn send_message_allows_gm_attributing_any_actor() {
        use crate::auth::role::ServerRole;
        use crate::data::command::UnsequencedCommand;
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
        let actor_id = Uuid::new_v4();
        repo.apply_command(UnsequencedCommand {
            world_id: w.id,
            author: player,
            ts: 0,
            ops: vec![Operation::Create {
                doc: seed_actor_doc(actor_id, w.id, Some(player)),
            }],
        })
        .await
        .unwrap();

        let ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::Actor { actor_id }),
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        // seq 2: the seeded actor doc's own Create consumed seq 1.
        assert_eq!(cmd.seq, 2, "GM may attribute a message to any actor doc");
    }

    #[tokio::test]
    async fn send_message_rejects_token_instance_attribution() {
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

        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::TokenInstance {
                token_id: Uuid::new_v4(),
            }),
            Audience::Public,
            100,
            30,
        )
        .await;
        assert!(
            matches!(err, Err(SendMessageError::ActorNotSpeakable)),
            "token-instance attribution has no first-party producer yet — rejected fail-closed"
        );
    }

    #[tokio::test]
    async fn send_message_rejects_attribution_to_a_non_actor_doc() {
        use crate::auth::role::ServerRole;
        use crate::data::command::UnsequencedCommand;
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
        let mut wrong_type = seed_actor_doc(Uuid::new_v4(), w.id, Some(player));
        wrong_type.doc_type = "note".into();
        let doc_id = wrong_type.id;
        repo.apply_command(UnsequencedCommand {
            world_id: w.id,
            author: player,
            ts: 0,
            ops: vec![Operation::Create { doc: wrong_type }],
        })
        .await
        .unwrap();

        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            &super::link_preview::build_client_allow_loopback(),
            &LinkPreviewCache::new(),
            &PreviewRateLimiter::new(),
            "all".into(),
            "grr".into(),
            Some(ActorOwnerRef::Actor { actor_id: doc_id }),
            Audience::Public,
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
    }
}

/// Ingest integration for the link-preview enrich stage (design doc §8):
/// drives `handle_send_message`/`handle_edit_message` directly, exactly like
/// `chat::tests`, but against a real stub `axum` target on `127.0.0.1` — the
/// same `allow_loopback` seam `link_preview.rs`'s own fetcher tests use. Kept
/// as its own `mod` (not folded into `chat::tests`) because every test here
/// needs `hyperlinks: true` and a stub server, unlike the rest of the file.
#[cfg(test)]
mod link_preview_ingest_tests {
    use super::*;
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;
    use axum::routing::get;
    use axum::Router;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Spawns a stub HTTP target on a random loopback port, returning the
    /// port. Mirrors `link_preview.rs`'s own test helper. Callers address it
    /// via the fake domain `stub.test` (never a literal `127.0.0.1` URL,
    /// which `validate_url` blocks unconditionally regardless of any
    /// loopback allowance) — `Fixture::new`'s client resolves that name to
    /// this stub's real loopback IP.
    async fn spawn_stub(router: Router) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    /// GM + one Player, `chat-settings` seeded with the given `policy` so the
    /// enrich stage's gating (`previews_enabled`) is under test control.
    struct Fixture {
        repo: SqliteRepository,
        room: std::sync::Arc<Room>,
        ctx: PermissionContext,
        rate: PingRateLimiter,
        preview_client: reqwest::Client,
        preview_cache: LinkPreviewCache,
        preview_rate: PreviewRateLimiter,
    }

    impl Fixture {
        async fn new(policy: ChatContentPolicy) -> Self {
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
            let gm_ctx = PermissionContext {
                user_id: gm,
                world_role: WorldRole::Gm,
            };
            let doc = Document {
                id: Uuid::new_v4(),
                scope: Scope::World { world_id: w.id },
                doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
                schema_version: 1,
                name: None,
                source: None,
                base: None,
                owner: Some(gm),
                permissions: PermissionSet::default(),
                embedded: BTreeMap::new(),
                parent_id: None,
                engine: Some(serde_json::to_value(policy).unwrap()),
                system: serde_json::json!({}),
                created_at: 0,
                updated_at: 0,
            };
            repo.apply_intent(
                &gm_ctx,
                w.id,
                vec![Operation::Create { doc }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

            let reg = RoomRegistry::new();
            let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
            let ctx = PermissionContext {
                user_id: player,
                world_role: WorldRole::Player,
            };
            Fixture {
                repo,
                room,
                ctx,
                rate: PingRateLimiter::new(),
                // Maps the fake domain `stub.test` to real loopback — the
                // ONLY way to reach a `127.0.0.1`-bound stub, since
                // `validate_url` rejects a literal `127.0.0.1` host outright
                // regardless of any loopback allowance (see that fn's doc).
                preview_client: link_preview::build_client_with_resolve_fn(|_host| {
                    Ok(vec!["127.0.0.1".parse().unwrap()])
                }),
                preview_cache: LinkPreviewCache::new(),
                preview_rate: PreviewRateLimiter::new(),
            }
        }

        async fn send(&self, content: &str, now: i64) -> Result<Command, SendMessageError> {
            handle_send_message(
                &self.room,
                &self.repo,
                &self.ctx,
                &self.rate,
                &self.preview_client,
                &self.preview_cache,
                &self.preview_rate,
                "all".into(),
                content.into(),
                None,
                Audience::Public,
                now,
                60,
            )
            .await
        }

        async fn edit(
            &self,
            message_id: Uuid,
            content: &str,
            now: i64,
        ) -> Result<Command, SendMessageError> {
            handle_edit_message(
                &self.room,
                &self.repo,
                &self.ctx,
                &self.rate,
                &self.preview_client,
                &self.preview_cache,
                &self.preview_rate,
                message_id,
                content.into(),
                now,
                60,
            )
            .await
        }

        async fn stored_engine(&self, cmd: &Command) -> MessageEngine {
            let doc_id = match &cmd.ops[0] {
                Operation::Create { doc } => doc.id,
                Operation::Update { doc_id, .. } => *doc_id,
                Operation::Delete { doc } => doc.id,
            };
            let doc = self.repo.get_document(doc_id).await.unwrap().unwrap();
            serde_json::from_value(doc.engine.unwrap()).unwrap()
        }
    }

    /// `markdown` must be on alongside `hyperlinks` for a plain `http://...`
    /// URL to actually become an `<a href>` — `sanitize`'s bare-toggle
    /// `hyperlinks` only controls whether ammonia PERMITS the `<a>` tag once
    /// produced; producing one at all needs markdown link syntax
    /// (`[label](url)`) run through `pulldown-cmark`. Bodies in this file
    /// use that syntax accordingly.
    fn hyperlinks_on() -> ChatContentPolicy {
        ChatContentPolicy {
            markdown: Some(true),
            hyperlinks: Some(true),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn normal_message_with_link_gets_trailing_preview() {
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async { axum::response::Html("<title>Hello</title>") }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let cmd = f
            .send(&format!("check out [link](http://stub.test:{addr}/)"), 1)
            .await
            .unwrap();
        let sys = f.stored_engine(&cmd).await;
        match sys.content.last() {
            Some(Segment::LinkPreview { url, title, .. }) => {
                assert!(url.contains(&addr.to_string()));
                assert_eq!(title, "Hello");
            }
            other => panic!("expected a trailing LinkPreview segment, got {other:?}"),
        }
        // The preview is APPENDED — the original Html run is still first.
        assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
    }

    #[tokio::test]
    async fn same_url_twice_hits_the_cache_one_fetch_total() {
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async {
                HITS.fetch_add(1, Ordering::SeqCst);
                axum::response::Html("<title>Cached</title>")
            }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let url = format!("http://stub.test:{addr}/");
        let first = f.send(&format!("one [x]({url})"), 1).await.unwrap();
        let second = f.send(&format!("two [x]({url})"), 2).await.unwrap();
        for cmd in [&first, &second] {
            let sys = f.stored_engine(cmd).await;
            assert!(matches!(
                sys.content.last(),
                Some(Segment::LinkPreview { .. })
            ));
        }
        assert_eq!(
            HITS.load(Ordering::SeqCst),
            1,
            "second send must hit the cache, not re-fetch"
        );
    }

    #[tokio::test]
    async fn max_previews_per_message_caps_four_links_to_three() {
        let addr = spawn_stub(Router::new().route(
            "/{id}",
            get(|| async { axum::response::Html("<title>P</title>") }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let body = format!(
            "[a](http://stub.test:{addr}/a) [b](http://stub.test:{addr}/b) [c](http://stub.test:{addr}/c) [d](http://stub.test:{addr}/d)"
        );
        let cmd = f.send(&body, 1).await.unwrap();
        let sys = f.stored_engine(&cmd).await;
        let preview_count = sys
            .content
            .iter()
            .filter(|s| matches!(s, Segment::LinkPreview { .. }))
            .count();
        assert_eq!(
            preview_count, MAX_PREVIEWS_PER_MESSAGE,
            "got {:?}",
            sys.content
        );
    }

    #[tokio::test]
    async fn failing_fetch_degrades_no_card_message_still_posts() {
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let cmd = f
            .send(&format!("broken [link](http://stub.test:{addr}/)"), 1)
            .await
            .unwrap();
        let sys = f.stored_engine(&cmd).await;
        assert!(
            !sys.content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "a failing fetch must not produce a card: {:?}",
            sys.content
        );
        assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
    }

    #[tokio::test]
    async fn previews_suppressed_when_hyperlinks_off() {
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async { axum::response::Html("<title>Nope</title>") }),
        ))
        .await;
        // Default policy: every toggle off, including hyperlinks.
        let f = Fixture::new(ChatContentPolicy::default()).await;
        let cmd = f
            .send(
                &format!("http://stub.test:{addr}/ plain text, no markup"),
                1,
            )
            .await
            .unwrap();
        let sys = f.stored_engine(&cmd).await;
        assert!(
            !sys.content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "hyperlinks off must suppress previews entirely: {:?}",
            sys.content
        );
    }

    #[tokio::test]
    async fn previews_suppressed_when_link_previews_explicitly_off() {
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async { axum::response::Html("<title>Nope</title>") }),
        ))
        .await;
        let f = Fixture::new(ChatContentPolicy {
            markdown: Some(true),
            hyperlinks: Some(true),
            link_previews: Some(false),
            ..Default::default()
        })
        .await;
        let cmd = f
            .send(&format!("check [link](http://stub.test:{addr}/)"), 1)
            .await
            .unwrap();
        let sys = f.stored_engine(&cmd).await;
        // Sanity: the link is still rendered as a real anchor — proving the
        // suppression below is `link_previews`-specific, not a side effect
        // of the link never becoming an `<a>` in the first place.
        assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
        assert!(
            !sys.content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "an explicit link_previews:false must suppress previews: {:?}",
            sys.content
        );
    }

    #[tokio::test]
    async fn burst_of_distinct_urls_hits_the_rate_limit_and_stops_fetching() {
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let addr = spawn_stub(Router::new().route(
            "/{id}",
            get(|| async {
                HITS.fetch_add(1, Ordering::SeqCst);
                axum::response::Html("<title>x</title>")
            }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        // A per-user PREVIEW_FETCH_PER_MIN budget of 20; drive far more than
        // that many DISTINCT-URL candidates (3 links/message, capped) in the
        // same 60s window so the limiter must reject some fetches outright.
        let mut total_previews = 0usize;
        for i in 0..15u32 {
            let body = format!(
                "[a](http://stub.test:{addr}/{i}a) [b](http://stub.test:{addr}/{i}b) [c](http://stub.test:{addr}/{i}c)"
            );
            let cmd = f.send(&body, 1_000).await.unwrap();
            let sys = f.stored_engine(&cmd).await;
            total_previews += sys
                .content
                .iter()
                .filter(|s| matches!(s, Segment::LinkPreview { .. }))
                .count();
        }
        // 15 messages * 3 distinct URLs each = 45 candidate fetches, but the
        // per-user budget is PREVIEW_FETCH_PER_MIN (20) within the window —
        // every message still posts (enrich degrades, never blocks the
        // send), but strictly fewer previews land than candidate URLs.
        assert!(
            total_previews < 45,
            "the rate limit must have stopped some fetches: {total_previews} previews from 45 candidates"
        );
        assert!(
            HITS.load(Ordering::SeqCst) <= PREVIEW_FETCH_PER_MIN,
            "no more real fetches than the per-user budget: {} hits",
            HITS.load(Ordering::SeqCst)
        );
    }

    /// A preview is derived, not authored — editing a message's links must
    /// re-derive the card set from the NEW content, dropping a stale preview
    /// whose link no longer appears.
    #[tokio::test]
    async fn edit_re_derives_preview_from_new_content() {
        let addr = spawn_stub(Router::new().route(
            "/",
            get(|| async { axum::response::Html("<title>Original</title>") }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let sent = f
            .send(&format!("see [link](http://stub.test:{addr}/)"), 1)
            .await
            .unwrap();
        let doc_id = match &sent.ops[0] {
            Operation::Create { doc } => doc.id,
            _ => unreachable!(),
        };
        let sys_before = f.stored_engine(&sent).await;
        assert!(matches!(
            sys_before.content.last(),
            Some(Segment::LinkPreview { .. })
        ));

        let edited = f.edit(doc_id, "no links here anymore", 2).await.unwrap();
        let sys_after = f.stored_engine(&edited).await;
        assert!(
            !sys_after
                .content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "an edit removing the link must drop the stale preview: {:?}",
            sys_after.content
        );
    }

    /// A `/roll` on a PREVIEW-ENABLED world never accumulates a `LinkPreview`
    /// segment: its content is exactly one `RollEmbed`. Pins the design §3
    /// EXPLICIT `kind != MessageKind::Roll` enrich guard — a successful roll
    /// falls through to the enrich gate (only the roll-execution-FAILURE arm
    /// returns early), so the guard, not the incidental absence of `<a href>`
    /// in a `RollEmbed`, is what keeps a roll message off the outbound-fetch
    /// path if the roll content model ever changes.
    #[tokio::test]
    async fn roll_message_never_gets_a_link_preview_even_when_previews_enabled() {
        let f = Fixture::new(hyperlinks_on()).await;
        let cmd = f.send("/roll 1d6", 1).await.unwrap();
        let sys = f.stored_engine(&cmd).await;
        assert_eq!(sys.kind, MessageKind::Roll);
        assert_eq!(
            sys.content.len(),
            1,
            "roll content must be one RollEmbed: {:?}",
            sys.content
        );
        assert!(matches!(
            sys.content.first(),
            Some(Segment::RollEmbed { .. })
        ));
        assert!(
            !sys.content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "a roll message must never carry a LinkPreview: {:?}",
            sys.content
        );
    }

    /// SECURITY: inert body text carrying a literal `href="..."` substring —
    /// NOT a markdown link, NOT inside an `<a>` tag — must never trigger a
    /// fetch. Markdown body text does not escape `"`/`'`, so this prose
    /// renders through `ammonia` unchanged as plain text; extraction must
    /// stay scoped to a genuine `<a>` tag span (see `extract_href_urls`'s
    /// doc), never a raw `href=` substring match anywhere in the run, or the
    /// server would fetch an attacker-chosen URL from invisible, non-
    /// hyperlink text. Proven at the ingest level (not just unit-level on
    /// `extract_href_urls`) via a call-counter stub: zero hits.
    #[tokio::test]
    async fn inert_href_substring_in_body_text_yields_no_preview_and_no_fetch() {
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let addr = spawn_stub(Router::new().route(
            "/x",
            get(|| async {
                HITS.fetch_add(1, Ordering::SeqCst);
                axum::response::Html("<title>should never be fetched</title>")
            }),
        ))
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let cmd = f
            .send(
                &format!(
                    "see href=\"http://stub.test:{addr}/x\" for details, no markdown link here"
                ),
                1,
            )
            .await
            .unwrap();
        let sys = f.stored_engine(&cmd).await;
        assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
        assert!(
            !sys.content
                .iter()
                .any(|s| matches!(s, Segment::LinkPreview { .. })),
            "inert body text with a literal href=\"...\" substring must not yield a preview: {:?}",
            sys.content
        );
        assert_eq!(
            HITS.load(Ordering::SeqCst),
            0,
            "the stub must never be hit for a non-anchor href substring"
        );
    }

    /// A stored pre-M11d-3 `MessageEngine` (no `LinkPreview` segments) still
    /// round-trips through the deserializer — the new segment variant is
    /// purely additive, not a breaking schema change.
    #[test]
    fn stored_pre_m11d3_message_still_deserializes() {
        let j = serde_json::json!({
            "channel": "all",
            "user_owner": Uuid::from_u128(1),
            "kind": "normal",
            "audience": { "kind": "public" },
            "content": [{ "kind": "text", "text": "hi" }],
        });
        let sys: MessageEngine = serde_json::from_value(j).unwrap();
        assert_eq!(sys.kind, MessageKind::Normal);
        assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
    }
}
