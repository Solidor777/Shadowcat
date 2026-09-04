//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with a typed, ingress-validated
//! `engine` body (this module's `MessageEngine`); `system` stays
//! reserved-empty (`{}`) for message docs. Authored and revised ONLY by the
//! server — never built or mutated by a client directly. A `message` doc_type reaches
//! `apply_intent` only via `handle_send_message` (Create), `handle_edit_message`,
//! or `handle_delete_message` (both Update, the latter a soft tombstone). Four
//! chokepoints jointly enforce this: the create-gate baseline-message exemption
//! (`apply_intent`'s `is_baseline_message` check, ties a Create to its
//! authenticated author); the ingress guard
//! (`ops_target_message`) rejects any client-authored `message` Create/Delete op
//! at the WS/HTTP boundary; `apply_intent`'s `Update` branch blanket-rejects
//! every client (`WriteOrigin::Client`) Update targeting a stored `message` doc
//! (Updates carry no `doc_type`, so they cannot be classified by
//! `ops_target_message` and must be blocked against the authoritative stored
//! document instead), exempting ONLY `WriteOrigin::ServerMessageRevision` — a
//! marker no wire frame can set, produced solely by `handle_edit_message`/
//! `handle_delete_message` after their own owner-or-GM check — and granting it a
//! scoped `Access` (`READ`+`WRITE_FIELDS` only, plus an exact-path admission in
//! `data::sqlite::apply_intent` for `/permissions/property_overrides` on a
//! message doc under this origin -- see `handle_recalc_roll` -- never any
//! other `/permissions` subpath, and never `/embedded`). `handle_recalc_roll`
//! is this origin's third producer, after its own GM-only check (never
//! owner-or-GM -- see `RecalcRollError::Forbidden`); the post-publish
//! enrichment republish (`chat::post_publish::run_pending_enrichments`) is a
//! fourth producer, after its own tombstone/OCC checks.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

mod commands;
mod host;
mod link_preview;
mod oembed;
mod post_publish;
mod preview_cache;
pub(crate) mod rolls;
mod sanitize;
mod settings;
mod shortcodes;
pub use commands::{parse_command, ParsedCommand};
pub use link_preview::{
    build_client as build_link_preview_client, enrich as enrich_link_previews, fetch_preview,
    LinkPreview, LinkPreviewDeps, PreviewError, MAX_PREVIEWS_PER_MESSAGE,
};
pub use oembed::{
    match_provider as match_oembed_provider, OEmbedProvider, OEmbedResponse, OEmbedSegment,
};
pub use post_publish::{
    run_pending_enrichments, PendingEnrichment, PostPublishDeps, PreviewFetchLocks,
};
pub use preview_cache::{
    LinkPreviewCache, PreviewRateLimiter, MAX_CACHE_ENTRIES, NEGATIVE_TTL, POSITIVE_TTL,
    PREVIEW_FETCH_PER_MIN,
};
pub use sanitize::sanitize;
pub use settings::{
    channel_registered, resolve_content_policy, resolve_dice_context, ChatContentPolicy,
    CHAT_SETTINGS_DOC_TYPE, DICE_SETTINGS_DOC_TYPE,
};

use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, Visibility, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::dice::rng::NoiseRng;
use crate::dice::{RawRoll, RecalcOp, RollOutcome, RollSpec};
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
/// — the legitimate message-edit/-delete path
/// (`handle_edit_message`/`handle_delete_message`), unreachable from any
/// client transport.
pub fn ops_target_message(ops: &[Operation]) -> bool {
    ops.iter().any(|op| match op {
        Operation::Create { doc } | Operation::Delete { doc } => doc.doc_type == MESSAGE_DOC_TYPE,
        // Like Update: the op carries only an id, so a Move targeting a stored
        // `message` doc is classified (and rejected) in `apply_intent`'s Move
        // branch against the authoritative stored `doc_type`.
        Operation::Update { .. } | Operation::Move { .. } => false,
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
    /// A canonical world-scoped actor document.
    Actor {
        /// The actor document id (world-pinned in `handle_send_message`).
        actor_id: Uuid,
    },
    /// An instanced actor addressed through its token.
    TokenInstance {
        /// The token document id the instanced actor lives on.
        token_id: Uuid,
    },
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
    /// Every world member may read — the unrestricted shape.
    #[default]
    Public,
    /// Only `recipients` (plus the sender) may read. The GM reads it ONLY if
    /// their own uuid is among `recipients` — not automatically.
    Whisper {
        /// User ids allowed to read (sender implicitly included).
        recipients: Vec<Uuid>,
    },
    /// Only whoever currently holds `WorldRole::Gm` (plus the sender) may
    /// read — resolved dynamically, not a frozen roster at send time.
    GmOnly,
}

/// Client-facing mirror of `dice::RecalcOp`, carried on the `RecalcRoll` wire
/// frame. `dice` is a pure library with no ts-rs bindings by design (see
/// `dice`'s crate doc) -- this type exists solely so a `RecalcOp` can ride
/// `ClientMsg`, converted via `into_recalc_op` before it ever reaches
/// `dice::recalc::recalculate`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireRecalcOp {
    /// Draw a fresh natural for each targeted die.
    RerollDice {
        /// Targeted die ids.
        ids: Vec<u32>,
    },
    /// Force a specific natural onto one die.
    ReplaceDie {
        /// The targeted die.
        id: u32,
        /// The natural face to force.
        natural: i32,
    },
    /// Drop targeted dice from their group's base naturals entirely.
    RemoveDice {
        /// Targeted die ids.
        ids: Vec<u32>,
    },
}

impl WireRecalcOp {
    /// Converts the wire shape into the dice engine's own `RecalcOp`.
    pub(crate) fn into_recalc_op(self) -> RecalcOp {
        match self {
            WireRecalcOp::RerollDice { ids } => RecalcOp::RerollDice(ids),
            WireRecalcOp::ReplaceDie { id, natural } => RecalcOp::ReplaceDie { id, natural },
            WireRecalcOp::RemoveDice { ids } => RecalcOp::RemoveDice(ids),
        }
    }
}

/// Message subtype, orthogonal to channel. Rides the opaque body (no ts-rs).
/// `Emote`/`Roll` are set by `parse_command`, `System` by server-authored
/// notices; a message carrying no command prefix stays `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Ordinary chat message.
    #[default]
    Normal,
    /// `/em`-style emote (rendered third-person).
    Emote,
    /// A message whose content came from the roll pipeline.
    Roll,
    /// Server-authored notice (never client-set).
    System,
}

/// One piece of a message's sanitized content model. Serialized into the
/// message's `engine` body (no ts-rs — the client declares its own Zod
/// mirror). Extensible: a new content type is added as a new `Segment`
/// variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text. Rendered as a DOM text node by the client (never innerHTML),
    /// so any markup it contains is inert.
    Text {
        /// The literal text (inert; rendered as a DOM text node).
        text: String,
    },
    /// A run of ammonia-sanitized HTML (safe by construction; the client renders
    /// it via innerHTML). Produced only by `chat::sanitize::sanitize`.
    Html {
        /// The ammonia-sanitized run (safe for innerHTML by construction).
        sanitized_html: String,
    },
    /// A completed roll: the formula plus its full deterministic outcome.
    /// `outcome` embeds the evaluated `RollOutcome` (records included -- the
    /// natural faces make the roll reproducible/auditable from the stored
    /// segment alone). `spec`/`raw` are kept (not discarded) so a GM can later
    /// recalculate this roll via `handle_recalc_roll`; `recalc_history` records
    /// every such recalculation. Produced only by `chat::rolls::execute_roll`,
    /// called from `handle_send_message`'s roll stage; a fresh embed is never
    /// produced on edit (rolls are immutable, see `handle_edit_message`) --
    /// `handle_recalc_roll` is the only path that ever mutates an existing one.
    RollEmbed {
        /// The formula as the author wrote it.
        formula: String,
        /// The full deterministic outcome, natural faces included. Overwritten by
        /// `handle_recalc_roll` on each recalculation; the PRE-recalc value is
        /// preserved as the newest `recalc_history` entry's `previous_outcome`.
        outcome: RollOutcome,
        /// Stable identity for this roll, independent of its position in `content`
        /// -- a recalc targets a roll by this id, never by array index, so it
        /// survives any future reordering (e.g. link-preview enrichment appending
        /// later segments). Defaults to a fresh id on deserialize so a roll
        /// embedded before this field existed still round-trips.
        #[serde(default = "Uuid::new_v4")]
        roll_id: Uuid,
        /// The parsed formula this roll was scored from, kept so a GM can later
        /// recalculate it. `None` for any roll embedded before this field existed
        /// -- `handle_recalc_roll` refuses `NoStoredState` on `None`, never
        /// guesses a spec back from `outcome`. GM-visible only (see
        /// `roll_embed_property_overrides`). Boxed: `RollSpec` is large enough
        /// that an unboxed `Option<RollSpec>` here would make `RollEmbed` the
        /// dominant variant of `Segment` by a wide margin
        /// (`clippy::large_enum_variant`); `Box` keeps the wire shape identical
        /// (serde serializes/deserializes `Box<T>` transparently as `T`) while
        /// moving the payload off every `Segment` value's own stack footprint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spec: Option<Box<RollSpec>>,
        /// The natural-face roll log `outcome` was evaluated from, kept for the
        /// same recalculation purpose as `spec` (same None-for-pre-existing rule
        /// and boxing rationale). GM-visible only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<Box<RawRoll>>,
        /// Present iff this roll has been recalculated at least once: an ordered,
        /// append-only audit log, each entry retaining the PRE-recalc
        /// `raw`/`outcome` it replaced -- the roll's original result is never
        /// silently discarded. Visible to every recipient (unlike `spec`/`raw`);
        /// each entry's OWN `previous_raw` is separately GM-gated.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recalc_history: Option<Vec<RecalcEntry>>,
    },
    /// An unexecuted, parse-and-cap-validated formula the card renders as a
    /// button; clicking it sends a fresh `/roll <formula>` `SendMessage`
    /// (a new, independently-attributed roll). `label` is plain data (never
    /// rendered as markup).
    RollButton {
        /// The validated-but-unexecuted formula the button re-sends.
        formula: String,
        /// Optional display label (plain data, never markup).
        label: Option<String>,
    },
    /// A server-fetched, SSRF-guarded preview of a link in the message.
    /// Rendered by the client from STORED data only — the client never
    /// fetches `url` or any remote resource (that would leak the viewer's
    /// IP). Appended at the END of `content` by `link_preview::enrich`,
    /// after every other segment the send/edit pipeline produced (`enrich`
    /// never reorders or removes an existing segment).
    LinkPreview {
        /// The previewed URL as posted.
        url: String,
        /// Server-extracted title.
        title: String,
        /// Server-extracted description (may be empty).
        description: String,
        /// The asset-ified `og:image`/canonical-image, once the post-publish
        /// background pipeline (`chat::post_publish`) has resolved one.
        /// Always `None` when `enrich` first appends this segment -- set
        /// later ONLY via a `WriteOrigin::ServerMessageRevision` republish
        /// (`run_pending_enrichments`), the same chokepoint
        /// `handle_edit_message`/`handle_delete_message` use.
        /// `#[serde(default)]`: every `LinkPreview` segment persisted before
        /// this field existed has no `image_asset_id` key on disk.
        #[serde(default)]
        image_asset_id: Option<Uuid>,
    },
    /// A provider-native embed from an ALLOWLISTED host (see `chat::oembed`'s
    /// module doc — no autodiscovery ever runs). STRUCTURED FIELDS ONLY: the
    /// provider's own `html` field never reaches this segment (see
    /// `OEmbedSegment`'s doc for the structural guarantee). A message whose
    /// posted URL matches the oEmbed allowlist gets exactly one `OEmbed`
    /// segment for that URL and no accompanying generic `LinkPreview` — the
    /// two are mutually exclusive per URL (`link_preview::enrich`).
    OEmbed(OEmbedSegment),
    /// A free-form, author-inserted link to a document or placed token, captured with its
    /// display label at authoring time (`label` is never re-resolved at render — only the
    /// fail-closed existence/visibility gate below re-checks `target`). Distinct from the
    /// actor-name header link, which is driven by `actor_owner` attribution, not body content.
    /// Produced by `chat::rolls::scan_body`'s `doc:`/`token:` prefix branch — reuses the SAME
    /// balanced `[[...]]` span mechanism as `RollEmbed`/`RollButton`, not a new one. No
    /// existence/visibility check runs against `target` at ingest: the CLIENT fails closed at
    /// render by checking `ctx.documents` presence for the target id (already redacted
    /// per-recipient by the normal document pipeline), the exact precedent the actor-name
    /// header link established.
    DocLink {
        /// What the link points at.
        target: DocLinkTarget,
        /// Display text captured at authoring time (the composer's `|<label>` span suffix).
        /// Rendering never re-resolves a live name lookup for this field.
        label: String,
    },
}

/// What a `Segment::DocLink` points at — mirrors the client's `SheetRef` shape (the
/// established "one anonymous cross-file-shared shape gets one name" precedent), given a
/// server-side equivalent since `SheetRef` itself is client-only TS. Carried inside
/// `Segment::DocLink`; parsed in full by `chat::rolls::scan_body`'s `doc:`/`token:` prefix
/// branch — `handle_send_message`'s ingest arm does no further parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocLinkTarget {
    /// A top-level document, optionally one level into an embedded child.
    Doc {
        /// The top-level document's id.
        doc_id: Uuid,
        /// A `/embedded/<collection>/<index>` pointer, one level deep, or `None` for the
        /// top-level document itself. Opaque to the server — never validated against the
        /// referenced document's actual `embedded` shape at ingest (the client's own
        /// `resolveDocRef` fails closed on a malformed/dangling pointer at open time).
        embedded_path: Option<String>,
    },
    /// A placed token, resolved client-side via its linked/embedded actor — the same
    /// resolution `ctx.openDocument` already performs for a `{tokenId}` `SheetRef`.
    Token {
        /// The placed token's document id.
        token_id: Uuid,
    },
}

/// One applied recalculation of a `RollEmbed`, appended to its `recalc_history`.
/// `previous_raw`/`previous_outcome` are the PRE-recalc state this entry
/// replaced -- the roll's live `raw`/`outcome` after the Nth entry is the Nth
/// entry's OUTPUT, which is either the (N+1)th entry's
/// `previous_raw`/`previous_outcome` or, for the last entry, the current
/// `RollEmbed.raw`/`RollEmbed.outcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecalcEntry {
    /// The targeted mutation(s) applied this recalculation.
    pub ops: Vec<RecalcOp>,
    /// The roll's natural-face log immediately BEFORE this recalculation.
    /// GM-visible only, same as `Segment::RollEmbed`'s `raw` field.
    pub previous_raw: RawRoll,
    /// The roll's outcome immediately BEFORE this recalculation. Visible to
    /// every recipient (not GM-gated) -- same visibility as `RollEmbed::outcome`.
    pub previous_outcome: RollOutcome,
    /// The GM who performed this recalculation.
    pub recalculated_by: Uuid,
    /// Epoch milliseconds this recalculation was applied -- this codebase's
    /// timestamp convention throughout `MessageEngine` (`edited_at`/`deleted_at`)
    /// and `Document` (`created_at`/`updated_at`) is an epoch-millisecond `i64`,
    /// never `chrono`, which is not a dependency anywhere in this crate; this
    /// field follows that established sibling-field convention rather than
    /// introducing a new dependency for one field.
    pub recalculated_at: i64,
}

/// Computes the `gm_only` `permissions.property_overrides` entries a
/// message's roll content requires: `spec`/`raw` on every `RollEmbed`, plus
/// `previous_raw` on every one of its `recalc_history` entries. Applied
/// uniformly to every `RollSpec`/`RawRoll`-shaped value under a `RollEmbed`
/// -- `outcome`/`previous_outcome`/`recalc_history` itself stay visible to
/// every recipient. Recomputed from scratch against the CURRENT `content`
/// (never incrementally patched), so a message's override set always matches
/// what it actually carries; called from `build_message_doc` at Create time
/// and from `handle_recalc_roll` after every recalculation.
pub(crate) fn roll_embed_property_overrides(content: &[Segment]) -> BTreeMap<String, Visibility> {
    let mut out = BTreeMap::new();
    for (i, seg) in content.iter().enumerate() {
        let Segment::RollEmbed {
            spec,
            raw,
            recalc_history,
            ..
        } = seg
        else {
            continue;
        };
        if spec.is_some() {
            out.insert(format!("/engine/content/{i}/spec"), Visibility::GmOnly);
        }
        if raw.is_some() {
            out.insert(format!("/engine/content/{i}/raw"), Visibility::GmOnly);
        }
        if let Some(history) = recalc_history {
            for j in 0..history.len() {
                out.insert(
                    format!("/engine/content/{i}/recalc_history/{j}/previous_raw"),
                    Visibility::GmOnly,
                );
            }
        }
    }
    out
}

/// The plain-text producer: wraps raw input as a single literal-text segment.
/// A richer producer (markdown/HTML) feeds this same content model.
pub fn plain_text_content(raw: &str) -> Vec<Segment> {
    vec![Segment::Text {
        text: raw.to_string(),
    }]
}

/// The message document's `engine` body. Opaque on the WIRE (no ts-rs — the
/// client declares its own Zod mirror, `ChatMessageEngine`/`parseMessageEngine`),
/// but ingress-validated server-side same as every other engine-defined
/// doc_type: `deny_unknown_fields` rejects any unknown key on this body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageEngine {
    /// Client-chosen channel label (never validated; audience never derives
    /// from it — see `Audience`).
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    /// Actor attribution, if the sender spoke as an actor (world-pinned and
    /// ownership-checked at send).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    /// Message subtype (normal/emote/roll/system).
    pub kind: MessageKind,
    /// Readership beyond world-readable; drives the doc's `PermissionSet`.
    #[serde(default)]
    pub audience: Audience,
    /// The sanitized segment list the client renders.
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
    /// EXPOSURE NOTE: like everything `index_content` sweeps — the `doc_type`,
    /// the envelope `name`, and every string and number leaf of `engine` and
    /// `system` — this pre-sanitize text is swept into the content-agnostic
    /// FTS index and can surface in `SearchHit.snippet`/`.document`. Any
    /// search-UI consumer must treat message-doc snippet/`source` strings as
    /// inert text (never innerHTML) — this field is the highest-volume
    /// raw-text instance of that pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Set by `handle_edit_message`. Absent (not `null`) on the wire for an
    /// unedited message, so a stored message carrying no marker still
    /// round-trips unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    /// Set by `handle_delete_message`'s soft tombstone. Absent (not `null`) on
    /// the wire for a live message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
}

/// A message's own fields, grouped apart from the writer identity
/// (`world_id`/`user`) and the moment (`now`) that `build_message_doc` takes
/// as separate parameters.
pub struct MessageDraft {
    /// Client-chosen display label; the server never validates or branches
    /// on it (see `Audience`'s doc for how a "GM" channel is actually
    /// enforced).
    pub channel: String,
    /// Attribution ref, ingest-validated by `handle_send_message` before
    /// this draft is built.
    pub actor_owner: Option<ActorOwnerRef>,
    /// Intended readership; drives the document's `PermissionSet` (see
    /// `build_message_doc`'s doc for the exact mapping).
    pub audience: Audience,
    /// Message subtype (`Normal`/`Emote`/`Roll`/`System`).
    pub kind: MessageKind,
    /// Sanitized/executed content segments.
    pub content: Vec<Segment>,
    /// Raw author input kept for client edit-prefill (see `MessageEngine::source`).
    pub source: Option<String>,
}

/// Server-construct a message `Document`. INVARIANT: only the server calls
/// this (via `handle_send_message`); clients never build message docs.
/// `draft.audience` drives the document's `PermissionSet`:
/// - `Public` — `default: Observer`, `gm_role: None` (the world-readable
///   shape; the GM's unconditional access is unaffected).
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
pub fn build_message_doc(world_id: Uuid, user: Uuid, draft: MessageDraft, now: i64) -> Document {
    let MessageDraft {
        channel,
        actor_owner,
        audience,
        kind,
        content,
        source,
    } = draft;
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
            property_overrides: roll_embed_property_overrides(&engine.content),
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
        MessageDraft {
            channel,
            actor_owner: None,
            audience: Audience::Whisper {
                recipients: vec![sender],
            },
            kind: MessageKind::System,
            content: vec![Segment::Text {
                text: err.to_string(),
            }],
            source: None,
        },
        now,
    )
}

/// Max characters accepted for a single message's raw content (pre-producer).
pub const MAX_MESSAGE_CHARS: usize = 4096;

/// Max characters accepted for a message's `channel` name. The one
/// declaration lives in `data::engine`
/// (`crate::data::engine::MAX_CHANNEL_CHARS`) so the channel registry's own
/// validation can read it without a layering inversion; re-exported here
/// where the ingest check uses it.
pub use crate::data::engine::MAX_CHANNEL_CHARS;

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
    /// The `channel` is not a key of the world's channel registry — refuse the send
    /// rather than file a message under (and select dice settings by) a
    /// channel that does not exist.
    UnknownChannel,
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
    /// The requested `actor_owner` cannot be attributed by this sender: for an `Actor` ref,
    /// the referenced doc does not exist, is not an `actor` doc_type, is outside the sending
    /// room's world, or is owned by someone else and the sender is not a GM; for a
    /// `TokenInstance` ref, the referenced doc does not exist, is not a `token` doc_type, is
    /// outside the sending room's world, or its effective owner (own override, else linked
    /// actor's owner) is someone else and the sender is not a GM.
    ActorNotSpeakable,
}

/// Player-presentable text for the sender's failure notice (correlated `ChatError`
/// wire frame). Classified per `[sec]`: only the SENDER'S OWN input / an immutable
/// product rule may be surfaced verbatim; every authorization-, existence-, or
/// internal-error-class variant collapses to a fixed generic string so a sender
/// cannot probe permission/ownership/existence structure through error text.
///
/// INVARIANT (no-leak): `Data`, `Forbidden`, `NotFound`, and `ActorNotSpeakable`
/// MUST ignore any inner value. `NotFound` and `Forbidden` are deliberately
/// IDENTICAL — distinguishing them is a message existence+ownership oracle.
impl std::fmt::Display for SendMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Validation-class: reveals only the sender's own input or a product rule.
            SendMessageError::Empty => f.write_str("Message cannot be empty."),
            SendMessageError::TooLong => f.write_str("Message is too long."),
            SendMessageError::RateLimited => {
                f.write_str("You are sending messages too quickly. Please wait a moment.")
            }
            SendMessageError::UnknownRecipient => {
                // Safe: the world roster is already member-visible (`list_members`),
                // so this discloses nothing a sender cannot already enumerate; the
                // offending id is never echoed.
                f.write_str("One or more whisper recipients are not members of this world.")
            }
            SendMessageError::UnknownChannel => {
                // Safe: the sender supplied the channel string, and the
                // registry's keys are already visible to every member through
                // the channel views.
                f.write_str("That channel does not exist.")
            }
            SendMessageError::AudienceLocked => {
                f.write_str("You cannot change who can see a message after it is sent.")
            }
            SendMessageError::RollImmutable => {
                f.write_str("A roll cannot be edited once it has been sent.")
            }
            // Authorization-class: generic only. The inner value is deliberately unused.
            SendMessageError::ActorNotSpeakable => {
                f.write_str("You are not permitted to send this message.")
            }
            SendMessageError::Forbidden | SendMessageError::NotFound => {
                f.write_str("You are not permitted to modify this message.")
            }
            // Internal error: generic; never leaks the inner DataError (SQL/constraint/path text).
            SendMessageError::Data(_) => {
                f.write_str("The message could not be delivered. Please try again.")
            }
            // Never surfaced here (caught upstream, authored as a System notice); kept
            // total + player-safe via RollError's own presentable Display.
            SendMessageError::Roll(e) => write!(f, "{e}"),
        }
    }
}

/// Why `handle_recalc_roll` refused a `RecalcRoll` frame.
#[derive(Debug)]
pub enum RecalcRollError {
    /// The requester holds no GM role in this world. Recalc is GM-only,
    /// audience-independent -- never owner-or-GM (see the module's design
    /// note on why there is no player self-service tier).
    Forbidden,
    /// The target message does not exist, or is not a `message` doc.
    NotFound,
    /// No `RollEmbed` in the message's content carries the given `roll_id`.
    RollNotFound,
    /// The targeted `RollEmbed` has no stored `spec`/`raw` -- it was embedded
    /// before this feature shipped and cannot be recalculated.
    NoStoredState,
    /// The user's per-minute flood budget is exhausted.
    RateLimited,
    /// The authoritative write failed.
    Data(DataError),
}

/// Player-presentable text for a `RecalcRoll` rejection (correlated
/// `ChatError`). `[sec]`-classified like `SendMessageError::Display`:
/// `Forbidden`/`NotFound`/`RollNotFound` collapse to one generic string (no
/// existence oracle); `NoStoredState` is safe to state exactly (recalc is
/// GM-only, so only an already-authorized GM ever sees it); `Data` never
/// leaks its inner detail.
impl std::fmt::Display for RecalcRollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecalcRollError::Forbidden
            | RecalcRollError::NotFound
            | RecalcRollError::RollNotFound => {
                f.write_str("You are not permitted to modify this message.")
            }
            RecalcRollError::NoStoredState => {
                f.write_str("This roll has no stored state to recalculate.")
            }
            RecalcRollError::RateLimited => {
                f.write_str("You are sending messages too quickly. Please wait a moment.")
            }
            RecalcRollError::Data(_) => {
                f.write_str("The message could not be delivered. Please try again.")
            }
        }
    }
}

/// Shared request-scoped dependencies for `handle_send_message`/
/// `handle_edit_message`: what the two entry points hold in common, grouped
/// the same way `LinkPreviewDeps` groups its own bundle of borrowed deps.
pub struct MessageRequestCtx<'a> {
    /// The world's room — the authoritative publish path.
    pub room: &'a Room,
    /// The document repository.
    pub repo: &'a dyn Repository,
    /// The caller's authenticated identity and world role.
    pub ctx: &'a PermissionContext,
    /// The per-user chat flood-budget limiter.
    pub rate: &'a PingRateLimiter,
    /// Link-preview fetch dependencies.
    pub preview: LinkPreviewDeps<'a>,
    /// The moment of this request (used for `created_at`/`edited_at`/rate accounting).
    pub now: i64,
    /// The per-user-per-minute flood budget.
    pub budget_per_min: usize,
}

/// Server-authoritative message ingest: flood-limit, validate, CONSTRUCT the
/// message doc, and publish it via the authoritative path. The sole message-
/// authoring entry point (see module-level INVARIANT comment) — a client can
/// only ever reach a stored `message` doc through this function.
pub async fn handle_send_message(
    req: MessageRequestCtx<'_>,
    channel: String,
    content: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
) -> Result<(Command, Vec<PendingEnrichment>), SendMessageError> {
    let MessageRequestCtx {
        room,
        repo,
        ctx,
        rate,
        preview,
        now,
        budget_per_min,
    } = req;
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
    // Channel membership gate: `channel` selects the per-channel dice
    // `ParseContext` and labels the clients' channel views, so an
    // unregistered channel is refused, not filed. Placed after the flood
    // check so the cheap guard stays ahead of the registry read.
    if !channel_registered(repo, room.world_id, &channel)
        .await
        .map_err(SendMessageError::Data)?
    {
        return Err(SendMessageError::UnknownChannel);
    }
    // Attribution ownership gate: `actor_owner` is client-supplied
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
                    // GM may attribute as any actor doc IN THIS WORLD; a
                    // Player only as one they own. A cross-world actor ref is
                    // refused at ingest, same as any other invalid ref — an
                    // actor doc's ownership grant does not cross world scope.
                    Some(d)
                        if d.doc_type == "actor"
                            && crate::data::document::world_of(d) == Some(room.world_id) =>
                    {
                        is_gm || d.owner == Some(ctx.user_id)
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(SendMessageError::ActorNotSpeakable);
                }
            }
            ActorOwnerRef::TokenInstance { token_id } => {
                let token_doc = repo
                    .get_document(*token_id)
                    .await
                    .map_err(SendMessageError::Data)?;
                let is_gm = ctx.world_role == WorldRole::Gm;
                let allowed = match &token_doc {
                    // Same world-pinning + GM-bypass shape as the `Actor` arm above.
                    // Ownership itself resolves through `effective_owner_of` — the
                    // repo-level chokepoint wrapping `permission::effective_owner` (a
                    // token's own `owner` override wins, else it inherits its linked
                    // actor's owner) — never reimplemented here.
                    Some(d)
                        if d.doc_type == crate::data::permission::TOKEN_DOC_TYPE
                            && crate::data::document::world_of(d) == Some(room.world_id) =>
                    {
                        is_gm
                            || repo
                                .effective_owner_of(d)
                                .await
                                .map_err(SendMessageError::Data)?
                                == Some(ctx.user_id)
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(SendMessageError::ActorNotSpeakable);
                }
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
    // Effective audience: an explicit /w wins; otherwise the `SendMessage`
    // frame's own `audience` field.
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
        let dice_ctx = resolve_dice_context(repo, room.world_id, &channel).await;
        // The roll's actor binding: the send's validated `actor_owner` —
        // references resolve against that document's `system` band, or fail
        // `unknown-ref` when nothing is bound.
        let host = match &actor_owner {
            Some(owner_ref) => host::host_for_actor_owner(repo, owner_ref)
                .await
                .map_err(SendMessageError::Data)?,
            None => None,
        };
        match rolls::execute_roll(&parsed.body, dice_ctx, host.as_ref()) {
            Ok((formula, outcome, spec, raw)) => vec![Segment::RollEmbed {
                formula,
                outcome,
                roll_id: Uuid::new_v4(),
                spec: Some(Box::new(spec)),
                raw: Some(Box::new(raw)),
                recalc_history: None,
            }],
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
                    .map(|cmd| (cmd, Vec::new()))
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
                    .map(|cmd| (cmd, Vec::new()))
                    .map_err(SendMessageError::Data);
            }
        };
        if let [rolls::BodyChunk::Text(_)] = chunks.as_slice() {
            sanitize(&parsed.body, &policy)
        } else {
            // Ambient dice context is resolved at most once, lazily, only when
            // a roll/button chunk actually appears in this body. The roll's
            // host (the send's actor binding) resolves just as lazily.
            let mut dice_ctx: Option<crate::dice::ParseContext> = None;
            let mut roll_host: Option<Option<Document>> = None;
            let mut segments = Vec::with_capacity(chunks.len());
            let mut roll_err = None;
            for chunk in chunks {
                match chunk {
                    rolls::BodyChunk::Text(t) => segments.extend(sanitize(t, &policy)),
                    rolls::BodyChunk::Inline(formula) => {
                        if dice_ctx.is_none() {
                            dice_ctx =
                                Some(resolve_dice_context(repo, room.world_id, &channel).await);
                        }
                        if roll_host.is_none() {
                            roll_host = Some(match &actor_owner {
                                Some(owner_ref) => host::host_for_actor_owner(repo, owner_ref)
                                    .await
                                    .map_err(SendMessageError::Data)?,
                                None => None,
                            });
                        }
                        let host_ref = roll_host.as_ref().expect("roll host computed").as_ref();
                        match rolls::execute_roll(formula, dice_ctx.unwrap(), host_ref) {
                            Ok((formula, outcome, spec, raw)) => {
                                segments.push(Segment::RollEmbed {
                                    formula,
                                    outcome,
                                    roll_id: Uuid::new_v4(),
                                    spec: Some(Box::new(spec)),
                                    raw: Some(Box::new(raw)),
                                    recalc_history: None,
                                })
                            }
                            Err(e) => {
                                roll_err = Some(e);
                                break;
                            }
                        }
                    }
                    rolls::BodyChunk::Button { formula, label } => {
                        if dice_ctx.is_none() {
                            dice_ctx =
                                Some(resolve_dice_context(repo, room.world_id, &channel).await);
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
                    rolls::BodyChunk::DocLink { target, label } => {
                        segments.push(Segment::DocLink {
                            target,
                            label: label.to_string(),
                        });
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
                    .map(|cmd| (cmd, Vec::new()))
                    .map_err(SendMessageError::Data);
            }
            segments
        }
    };
    let mut pending: Vec<PendingEnrichment> = Vec::new();
    // Link-preview enrich stage: only for hyperlink-carrying, non-Roll bodies.
    // The `kind != Roll` guard is EXPLICIT, not incidental: a
    // successful roll falls through here with `content_segments == [RollEmbed]`
    // (only the roll-EXECUTION-FAILURE arm returns early), so without this
    // guard a `/roll` on a preview-enabled world would enter `enrich` — a no-op
    // today only because `enrich` scans `Segment::Html` runs (none in a
    // RollEmbed), but a latent path to attaching outbound-fetched previews to a
    // roll message if that ever changes. Synchronous, before publish — no
    // spawned task, no post-publish revision, no message-deleted-mid-fetch race.
    if parsed.kind != MessageKind::Roll && policy.previews_enabled() {
        pending = link_preview::enrich(
            &mut content_segments,
            link_preview::EnrichDeps {
                repo,
                fetch: preview,
            },
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
        MessageDraft {
            channel,
            actor_owner,
            audience,
            kind: parsed.kind,
            content: content_segments,
            source,
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
    .map(|cmd| (cmd, pending))
    .map_err(SendMessageError::Data)
}

/// Server-authoritative message edit: owner-or-GM only, and rewrites ONLY
/// `content`/`kind`/`edited_at` on the stored `/engine` body —
/// `channel`/`user_owner`/`actor_owner`/`audience`/`deleted_at` are copied
/// verbatim from the stored document, never re-derived from the edit
/// request.
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
pub async fn handle_edit_message(
    req: MessageRequestCtx<'_>,
    message_id: Uuid,
    content: String,
) -> Result<(Command, Vec<PendingEnrichment>), SendMessageError> {
    let MessageRequestCtx {
        room,
        repo,
        ctx,
        rate,
        preview,
        now,
        budget_per_min,
    } = req;
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
    // front-door set the audience (the `SendMessage` frame's own `audience`
    // field, or a content `/w`) — a
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
    let mut pending: Vec<PendingEnrichment> = Vec::new();
    if policy.previews_enabled() {
        pending = link_preview::enrich(
            &mut segments,
            link_preview::EnrichDeps {
                repo,
                fetch: preview,
            },
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
            remove: false,
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
        }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map(|cmd| (cmd, pending))
        .map_err(SendMessageError::Data)
}

/// Extracts the message doc id a `Command` from `handle_send_message` (a
/// `Create`) or `handle_edit_message` (an `Update`) targeted — the id the
/// post-publish background pipeline republishes against.
pub fn command_message_id(cmd: &Command) -> Option<Uuid> {
    match cmd.ops.first()? {
        Operation::Create { doc } => Some(doc.id),
        Operation::Update { doc_id, .. } => Some(*doc_id),
        Operation::Delete { .. } | Operation::Move { .. } => None,
    }
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
            remove: false,
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
        }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(SendMessageError::Data)
}

/// Shared request-scoped dependencies for `handle_recalc_roll`: the same
/// room/repo/ctx/rate/now/budget grouping `MessageRequestCtx` uses for
/// `handle_send_message`/`handle_edit_message`, minus `preview` (recalc never
/// touches link-preview enrichment) -- kept as its own struct rather than
/// reusing `MessageRequestCtx` so this call site never threads an unused
/// field. Grouped (instead of nine positional parameters) to stay under
/// `clippy::too_many_arguments` by restructuring the signature, never by
/// suppressing the lint.
pub struct RecalcRollRequestCtx<'a> {
    /// The world's room -- the authoritative publish path.
    pub room: &'a Room,
    /// The document repository.
    pub repo: &'a dyn Repository,
    /// The caller's authenticated identity and world role.
    pub ctx: &'a PermissionContext,
    /// The per-user chat flood-budget limiter.
    pub rate: &'a PingRateLimiter,
    /// The moment of this request (used for `recalculated_at`/rate accounting).
    pub now: i64,
    /// The per-user-per-minute flood budget.
    pub budget_per_min: usize,
}

/// Server-authoritative roll correction: GM-only (never owner-or-GM -- see
/// `RecalcRollError::Forbidden`), locates the targeted `RollEmbed` by
/// `roll_id` (never by array index), re-derives it via `dice::recalculate`,
/// and appends an auditable `RecalcEntry` capturing the PRE-recalc
/// `raw`/`outcome` before overwriting them. Reuses
/// `WriteOrigin::ServerMessageRevision` -- the SAME chokepoint
/// `handle_edit_message`/`handle_delete_message` use -- as its third caller.
/// Also writes `/permissions/property_overrides` in the SAME
/// `Operation::Update`, which `apply_intent`'s `ServerMessageRevision` branch
/// admits ONLY at that exact path (see `data::sqlite::apply_intent`'s
/// exact-path admission) -- needed because a freshly-appended
/// `RecalcEntry.previous_raw` pointer must be added to the GM-only override
/// set on every recalc.
pub async fn handle_recalc_roll(
    req: RecalcRollRequestCtx<'_>,
    message_id: Uuid,
    roll_id: Uuid,
    ops: Vec<RecalcOp>,
) -> Result<Command, RecalcRollError> {
    let RecalcRollRequestCtx {
        room,
        repo,
        ctx,
        rate,
        now,
        budget_per_min,
    } = req;
    if ctx.world_role != WorldRole::Gm {
        return Err(RecalcRollError::Forbidden);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(RecalcRollError::RateLimited);
    }
    let cur = repo
        .get_document(message_id)
        .await
        .map_err(RecalcRollError::Data)?
        .ok_or(RecalcRollError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return Err(RecalcRollError::NotFound);
    }
    let mut sys: MessageEngine = serde_json::from_value(cur.engine.clone().unwrap_or_default())
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;

    let idx = sys
        .content
        .iter()
        .position(|seg| matches!(seg, Segment::RollEmbed { roll_id: rid, .. } if *rid == roll_id))
        .ok_or(RecalcRollError::RollNotFound)?;

    // First pass: read immutably and clone what's needed, so the mutation
    // below never overlaps a live borrow.
    let (spec_val, raw_val, prev_outcome) = match &sys.content[idx] {
        Segment::RollEmbed {
            spec: Some(s),
            raw: Some(r),
            outcome,
            ..
        } => (s.clone(), r.clone(), outcome.clone()),
        Segment::RollEmbed { .. } => return Err(RecalcRollError::NoStoredState),
        _ => unreachable!("idx matched only Segment::RollEmbed above"),
    };

    let seed = rolls::entropy_seed();
    let mut rng = NoiseRng::from_seed(seed);
    let (new_raw, new_outcome) = crate::dice::recalculate(&spec_val, &raw_val, &ops, &mut rng);

    let entry = RecalcEntry {
        ops,
        previous_raw: *raw_val,
        previous_outcome: prev_outcome,
        recalculated_by: ctx.user_id,
        recalculated_at: now,
    };
    if let Segment::RollEmbed {
        raw,
        outcome,
        recalc_history,
        ..
    } = &mut sys.content[idx]
    {
        recalc_history.get_or_insert_with(Vec::new).push(entry);
        *raw = Some(Box::new(new_raw));
        *outcome = new_outcome;
    }

    let overrides = roll_embed_property_overrides(&sys.content);
    let new_engine = serde_json::to_value(&sys)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;
    let new_overrides_json = serde_json::to_value(&overrides)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;
    let old_overrides_json = serde_json::to_value(&cur.permissions.property_overrides)
        .map_err(|e| RecalcRollError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![
            FieldChange {
                remove: false,
                path: "/engine".into(),
                old: cur.engine.clone().unwrap_or_default(),
                new: new_engine,
            },
            FieldChange {
                remove: false,
                path: "/permissions/property_overrides".into(),
                old: old_overrides_json,
                new: new_overrides_json,
            },
        ],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(RecalcRollError::Data)
}

#[cfg(test)]
mod tests;

/// Ingest integration for the link-preview enrich stage:
/// drives `handle_send_message`/`handle_edit_message` directly, exactly like
/// `chat::tests`, but against a real stub `axum` target on `127.0.0.1` — the
/// same `build_client_allow_loopback` seam `link_preview`'s own fetcher tests use. Kept
/// as its own `mod` (not folded into `chat::tests`) because every test here
/// needs `hyperlinks: true` and a stub server, unlike the tests in `chat::tests`.
#[cfg(test)]
mod link_preview_ingest_tests;
