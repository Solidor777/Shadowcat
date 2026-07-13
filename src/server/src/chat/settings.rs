//! Per-world chat content policy: a single `chat-settings` config `Document`
//! read by the message sanitizer to decide which enrichment producers are
//! allowed. Resolution is fail-closed: an absent doc, a query error, or a
//! `system` body that does not deserialize into `ChatContentPolicy` all yield
//! `ChatContentPolicy::default()` (every toggle off, i.e. plain text). The
//! toggles only ever WIDEN enrichment from that safe baseline, so any failure
//! mode degrades safe rather than open.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::repository::Repository;
use crate::dice::{Direction, ModeKind, ParseContext};

/// Doc_type for the single per-world chat-settings config `Document`.
pub const CHAT_SETTINGS_DOC_TYPE: &str = "chat-settings";

/// GM-configured chat content policy, stored as the `system` body of the
/// `chat-settings` doc. Every field defaults `false`. `#[serde(default)]` on
/// the struct means a partial body (only some fields set) fills the rest with
/// `false` rather than failing deserialization — a partial policy still
/// degrades safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatContentPolicy {
    pub markdown: bool,
    pub html: bool,
    pub images: bool,
    pub hyperlinks: bool,
    pub emails: bool,
}

/// Read the world's chat content policy, fail-closed. A query error, an
/// absent `chat-settings` doc, or a `system` body that fails to deserialize
/// into `ChatContentPolicy` all yield `ChatContentPolicy::default()`.
pub async fn resolve_content_policy(repo: &dyn Repository, world_id: Uuid) -> ChatContentPolicy {
    let docs = match repo.query_documents(world_id, CHAT_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return ChatContentPolicy::default(),
    };
    let Some(doc) = docs.into_iter().next() else {
        return ChatContentPolicy::default();
    };
    serde_json::from_value(doc.system).unwrap_or_default()
}

/// Doc_type for the single per-world dice-settings config `Document`.
pub const DICE_SETTINGS_DOC_TYPE: &str = "dice-settings";

/// Wire-shape mirror of `dice::ModeKind`, stored in the `dice-settings` body.
/// Kept separate from `ModeKind` (no serde derive there — the `dice` crate is
/// pure, no wire coupling) rather than deriving serde directly on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiceModeSetting {
    #[default]
    Total,
    SuccessCount,
}

/// Wire-shape mirror of `dice::Direction`, stored in the `dice-settings` body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiceDirectionSetting {
    #[default]
    HighWins,
    LowWins,
}

/// GM-configured ambient dice-notation context, stored as the `system` body of
/// the `dice-settings` doc. `#[serde(default)]` on both fields (and the struct)
/// means a partial or absent body fills the rest with the safe default
/// (Total + HighWins) rather than failing deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
struct DiceSettingsBody {
    mode: DiceModeSetting,
    direction: DiceDirectionSetting,
}

/// Read the world's ambient dice-notation `ParseContext`, fail-closed. A query
/// error, an absent `dice-settings` doc, or a `system` body that fails to
/// deserialize into `DiceSettingsBody` all yield `ParseContext { mode: Total,
/// direction: HighWins }` — the same safe baseline `resolve_content_policy`
/// uses for chat enrichment.
pub async fn resolve_dice_context(repo: &dyn Repository, world: Uuid) -> ParseContext {
    let default = ParseContext::default();
    let docs = match repo.query_documents(world, DICE_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return default,
    };
    let Some(doc) = docs.into_iter().next() else {
        return default;
    };
    let body: DiceSettingsBody = match serde_json::from_value(doc.system) {
        Ok(b) => b,
        Err(_) => return default,
    };
    ParseContext {
        mode: match body.mode {
            DiceModeSetting::Total => ModeKind::Total,
            DiceModeSetting::SuccessCount => ModeKind::SuccessCount,
        },
        direction: match body.direction {
            DiceDirectionSetting::HighWins => Direction::HighWins,
            DiceDirectionSetting::LowWins => Direction::LowWins,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::role::ServerRole;
    use crate::data::command::{Operation, WriteOrigin};
    use crate::data::document::{Document, PermissionSet, Scope, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;
    use std::collections::BTreeMap;

    /// A fresh in-memory world with a GM, for chat-settings resolution tests.
    async fn world() -> (SqliteRepository, Uuid, Uuid) {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        (repo, w.id, gm)
    }

    /// A `chat-settings` `Document` in `world_id` with the given `system` body,
    /// owned by `gm`.
    fn settings_doc(world_id: Uuid, gm: Uuid, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id },
            doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            source: None,
            owner: Some(gm),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn default_policy_is_all_off() {
        let p = ChatContentPolicy::default();
        assert!(!p.markdown && !p.html && !p.images && !p.hyperlinks && !p.emails);
    }

    #[tokio::test]
    async fn absent_settings_doc_resolves_to_default() {
        let (repo, world_id, _gm) = world().await;
        assert_eq!(
            resolve_content_policy(&repo, world_id).await,
            ChatContentPolicy::default()
        );
    }

    #[tokio::test]
    async fn malformed_settings_body_resolves_to_default() {
        let (repo, world_id, gm) = world().await;
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        // `markdown` is a type mismatch (string, not bool), so
        // `serde_json::from_value::<ChatContentPolicy>` errors even with
        // `#[serde(default)]` — a merely-missing field would NOT error.
        let doc = settings_doc(
            world_id,
            gm,
            serde_json::json!({ "markdown": "not-a-bool" }),
        );
        repo.apply_intent(
            &gm_ctx,
            world_id,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
        assert_eq!(
            resolve_content_policy(&repo, world_id).await,
            ChatContentPolicy::default()
        );
    }

    #[tokio::test]
    async fn present_policy_is_read() {
        let (repo, world_id, gm) = world().await;
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let doc = settings_doc(
            world_id,
            gm,
            serde_json::json!({
                "markdown": true, "html": false, "images": true,
                "hyperlinks": true, "emails": false
            }),
        );
        repo.apply_intent(
            &gm_ctx,
            world_id,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
        let p = resolve_content_policy(&repo, world_id).await;
        assert!(p.markdown && p.images && p.hyperlinks && !p.html && !p.emails);
    }

    /// A `dice-settings` `Document` in `world_id` with the given `system`
    /// body, owned by `gm`.
    fn dice_settings_doc(world_id: Uuid, gm: Uuid, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id },
            doc_type: DICE_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            source: None,
            owner: Some(gm),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    async fn create_dice_doc(repo: &SqliteRepository, world_id: Uuid, gm: Uuid, doc: Document) {
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        repo.apply_intent(
            &gm_ctx,
            world_id,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    #[test]
    fn default_dice_context_is_total_high_wins() {
        let ctx = ParseContext::default();
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn absent_dice_settings_doc_resolves_to_default() {
        let (repo, world_id, _gm) = world().await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn malformed_dice_settings_body_resolves_to_default() {
        let (repo, world_id, gm) = world().await;
        // `mode` is a type mismatch (number, not a known string), so
        // deserialization into `DiceSettingsBody` errors outright.
        let doc = dice_settings_doc(world_id, gm, serde_json::json!({ "mode": 5 }));
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn unknown_enum_variant_string_resolves_to_default() {
        let (repo, world_id, gm) = world().await;
        // An out-of-vocabulary variant string (not a type mismatch) also fails
        // the whole-body deserialization — no #[serde(other)] catch-all exists,
        // so fail-closed covers this distinct failure class too.
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "foobar", "direction": "low_wins" }),
        );
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn total_high_wins_is_read() {
        let (repo, world_id, gm) = world().await;
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "total", "direction": "high_wins" }),
        );
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn total_low_wins_is_read() {
        let (repo, world_id, gm) = world().await;
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "total", "direction": "low_wins" }),
        );
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::LowWins);
    }

    #[tokio::test]
    async fn success_count_high_wins_is_read() {
        let (repo, world_id, gm) = world().await;
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "success_count", "direction": "high_wins" }),
        );
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::SuccessCount);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn success_count_low_wins_is_read() {
        let (repo, world_id, gm) = world().await;
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "success_count", "direction": "low_wins" }),
        );
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::SuccessCount);
        assert_eq!(ctx.direction, Direction::LowWins);
    }

    #[tokio::test]
    async fn partial_body_defaults_the_other_field() {
        let (repo, world_id, gm) = world().await;
        // Only `mode` set; `direction` must default to `HighWins`.
        let doc = dice_settings_doc(world_id, gm, serde_json::json!({ "mode": "success_count" }));
        create_dice_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::SuccessCount);
        assert_eq!(ctx.direction, Direction::HighWins);

        // Only `direction` set; `mode` must default to `Total`.
        let (repo2, world_id2, gm2) = world().await;
        let doc2 = dice_settings_doc(
            world_id2,
            gm2,
            serde_json::json!({ "direction": "low_wins" }),
        );
        create_dice_doc(&repo2, world_id2, gm2, doc2).await;
        let ctx2 = resolve_dice_context(&repo2, world_id2).await;
        assert_eq!(ctx2.mode, ModeKind::Total);
        assert_eq!(ctx2.direction, Direction::LowWins);
    }
}
