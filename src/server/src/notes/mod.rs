//! This module relays the GM's "present this note to the table" focus signal.
//!
//! `handle_present_note` authors no document: it validates a `ClientMsg::PresentNote` and
//! broadcasts `ServerMsg::PresentNote` out-of-band through `Room::broadcast_aux`, the same shape
//! `ServerMsg::Vfx` uses. It is a sibling of `tables` because it is chat-adjacent but not chat.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::document::{world_of, DocRole, WorldRole};
use crate::data::engine::NOTE_DOC_TYPE;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::protocol::ServerMsg;
use crate::ws::room::Room;
use crate::ws::PingRateLimiter;

/// Why a `PresentNote` request was refused. Every `Display` is player-presentable text carried
/// by `ServerMsg::ChatError`; `Forbidden` and `Data` reveal nothing about the note.
///
/// # Examples
///
/// ```
/// use shadowcat::notes::PresentNoteError;
///
/// assert_eq!(
///     PresentNoteError::NotShared.to_string(),
///     "Share this note with players before presenting it."
/// );
/// ```
#[derive(Debug)]
pub enum PresentNoteError {
    /// The caller is not the world's GM.
    Forbidden,
    /// The caller's shared per-minute VFX budget is exhausted.
    RateLimited,
    /// The id does not name a `note` document in this world.
    NotFound,
    /// The note's `permissions.default` is `DocRole::None`: players cannot read it, and a
    /// presentation never widens access (the GM shares it with an ordinary `Update` first).
    NotShared,
    /// A repository error.
    Data(crate::data::DataError),
}

impl std::fmt::Display for PresentNoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PresentNoteError::Forbidden => "Only the GM can present a note.",
            PresentNoteError::RateLimited => {
                "You are presenting too quickly. Please wait a moment."
            }
            PresentNoteError::NotFound => "That note could not be found.",
            PresentNoteError::NotShared => "Share this note with players before presenting it.",
            PresentNoteError::Data(_) => "That note could not be presented.",
        })
    }
}

impl std::error::Error for PresentNoteError {}

/// Validates one `PresentNote` request and, on success, broadcasts `ServerMsg::PresentNote` to
/// every connected member of `room`.
///
/// Order: GM gate (a non-GM spends no budget), then the shared `vfx_rate` bucket at
/// `VFX_RATE_PER_MIN` (the bucket `/fx` and `PlayVfx` spend, so a present spam is limited like a
/// VFX spam), then the note read. The note must exist, be a `note` of THIS world, and grant
/// players a role above `DocRole::None`: the broadcast never reaches around permissions, so a
/// recipient who still cannot read the note simply fails to open it.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::membership::PermissionContext;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::notes::handle_present_note;
/// use shadowcat::ws::room::RoomRegistry;
/// use shadowcat::ws::PingRateLimiter;
/// use uuid::Uuid;
///
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let registry = RoomRegistry::new();
/// let room = registry.get_or_create(&repo, Uuid::new_v4()).await.unwrap().unwrap();
/// let ctx = PermissionContext { user_id: Uuid::new_v4(), world_role: WorldRole::Player };
/// let rate = PingRateLimiter::new();
/// // A player is refused before any budget or document read.
/// assert!(handle_present_note(&room, &repo, &ctx, &rate, Uuid::new_v4(), 0).await.is_err());
/// # }
/// ```
pub async fn handle_present_note(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    vfx_rate: &PingRateLimiter,
    note_id: Uuid,
    now: i64,
) -> Result<(), PresentNoteError> {
    if ctx.world_role != WorldRole::Gm {
        return Err(PresentNoteError::Forbidden);
    }
    if !vfx_rate.check(ctx.user_id, now, crate::ws::VFX_RATE_PER_MIN) {
        return Err(PresentNoteError::RateLimited);
    }
    let doc = repo
        .get_document(note_id)
        .await
        .map_err(PresentNoteError::Data)?
        .filter(|d| d.doc_type == NOTE_DOC_TYPE && world_of(d) == Some(room.world_id))
        .ok_or(PresentNoteError::NotFound)?;
    if doc.permissions.default == DocRole::None {
        return Err(PresentNoteError::NotShared);
    }
    room.broadcast_aux(ServerMsg::PresentNote {
        note_id,
        presented_by: ctx.user_id,
    });
    Ok(())
}

#[cfg(test)]
mod tests;
