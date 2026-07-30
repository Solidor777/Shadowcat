//! Per-world chat content policy: a single `chat-settings` config `Document`
//! read by the message sanitizer to decide which enrichment producers are
//! allowed. Resolution is fail-closed: an absent doc, a query error, or an
//! `engine` body that does not deserialize into `ChatSettingsEngine` all
//! yield `ChatContentPolicy::default()` (every toggle absent, i.e. plain
//! text). The toggles only ever WIDEN enrichment from that safe baseline, so
//! any failure mode degrades safe rather than open.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::engine::{
    ChatSettingsEngine, DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine,
};
use crate::data::repository::Repository;
use crate::dice::{Direction, ModeKind, ParseContext};

/// Doc_type for the single per-world chat-settings config `Document`.
pub const CHAT_SETTINGS_DOC_TYPE: &str = "chat-settings";

/// GM-configured chat content policy, stored as the `engine` body of the
/// `chat-settings` doc (M13-0: re-rooted from `system`). The ONE definition —
/// `data::engine::registries::ChatSettingsEngine` (the ts-rs-exported, ingress-
/// validated struct) — re-exported under this chat-domain name rather than
/// duplicated. Every field is `Option<bool>`; absent (`None`) resolves to
/// `false` via the accessor methods below, EXCEPT `link_previews`, whose
/// tri-state semantics are documented on `previews_enabled`.
pub type ChatContentPolicy = ChatSettingsEngine;

impl ChatContentPolicy {
    /// Markdown rendering allowed (absent = false, fail-closed).
    pub fn markdown(&self) -> bool {
        self.markdown.unwrap_or(false)
    }
    /// Raw-HTML input allowed (absent = false; output still ammonia-cleaned).
    pub fn html(&self) -> bool {
        self.html.unwrap_or(false)
    }
    /// Image embeds allowed (absent = false).
    pub fn images(&self) -> bool {
        self.images.unwrap_or(false)
    }
    /// Hyperlink anchors allowed (absent = false).
    pub fn hyperlinks(&self) -> bool {
        self.hyperlinks.unwrap_or(false)
    }
    /// Email autolinks allowed (absent = false).
    pub fn emails(&self) -> bool {
        self.emails.unwrap_or(false)
    }

    /// Resolved link-preview enablement (design doc §6): previews require
    /// `hyperlinks` to be on (a preview with no rendered link is
    /// meaningless), and within that, `link_previews` defaults ON when
    /// absent — a GM must explicitly write `link_previews: false` to opt
    /// out once hyperlinks are enabled. A fail-closed empty/default policy
    /// (`hyperlinks` absent) always resolves to `false` regardless of
    /// `link_previews`.
    pub fn previews_enabled(&self) -> bool {
        self.hyperlinks() && self.link_previews.unwrap_or(true)
    }
}

/// Read the world's chat content policy, fail-closed. A query error, an
/// absent `chat-settings` doc, or an `engine` body that fails to deserialize
/// into `ChatContentPolicy` all yield `ChatContentPolicy::default()`.
///
/// SINGLETON RESOLUTION: `chat-settings` is a per-world singleton, but nothing
/// yet enforces uniqueness at the create chokepoint (the GM editor's seed guard
/// is client-side only). Resolution is DETERMINISTIC regardless: `query_documents`
/// orders `ORDER BY id`, so if two `chat-settings` docs ever coexist the
/// lowest-UUID one always wins — never a nondeterministic policy. The fail-closed
/// direction bounds a stray doc (it can only WIDEN enrichment, which still needs
/// GM-authored content to matter). Construction-time uniqueness (a singleton
/// doc-type create-gate) is the stronger, still-deferred half — see `docs/TODO.md`.
pub async fn resolve_content_policy(repo: &dyn Repository, world_id: Uuid) -> ChatContentPolicy {
    let docs = match repo.query_documents(world_id, CHAT_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return ChatContentPolicy::default(),
    };
    let Some(doc) = docs.into_iter().next() else {
        return ChatContentPolicy::default();
    };
    doc.engine
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Doc_type for the single per-world dice-settings config `Document`.
pub const DICE_SETTINGS_DOC_TYPE: &str = "dice-settings";

/// Read the world's ambient dice-notation `ParseContext`, fail-closed. A query
/// error, an absent `dice-settings` doc, or an `engine` body that fails to
/// deserialize into `DiceSettingsEngine` all yield `ParseContext { mode: Total,
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
    let body: DiceSettingsEngine = match doc.engine.and_then(|v| serde_json::from_value(v).ok()) {
        Some(b) => b,
        None => return default,
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
    use crate::data::document::{Document, PermissionSet, Scope};
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

    /// A `chat-settings` `Document` in `world_id` with the given `engine`
    /// body, owned by `gm`. `system` stays empty — `chat-settings` is
    /// engine-defined, and only `resolve_content_policy`'s `engine` read
    /// matters here.
    fn settings_doc(world_id: Uuid, gm: Uuid, engine: serde_json::Value) -> Document {
        Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id },
            doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: Some(gm),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            engine: Some(engine),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Seed a `chat-settings`/`dice-settings` doc via the test-only raw
    /// insert (`SqliteRepository::seed_document_unvalidated`) — bypasses
    /// the `validate_engine_tree` ingress gate (which `apply_command` now
    /// runs too, same as `apply_intent`), so a deliberately malformed
    /// `engine` body can still be persisted to exercise
    /// `resolve_content_policy`/`resolve_dice_context`'s own runtime
    /// fail-closed fallback (a well-formed body would never reach a
    /// malformed-engine test case if the ingress gate rejected the Create
    /// outright before it was ever stored).
    async fn seed_settings_doc(repo: &SqliteRepository, _world_id: Uuid, _gm: Uuid, doc: Document) {
        repo.seed_document_unvalidated(&doc).await.unwrap();
    }

    #[test]
    fn default_policy_is_all_off() {
        let p = ChatContentPolicy::default();
        assert!(!p.markdown() && !p.html() && !p.images() && !p.hyperlinks() && !p.emails());
        assert_eq!(p.link_previews, None);
        assert!(
            !p.previews_enabled(),
            "hyperlinks off must yield previews disabled regardless of link_previews"
        );
    }

    #[test]
    fn previews_enabled_hyperlinks_off_is_always_false() {
        let mut p = ChatContentPolicy {
            hyperlinks: Some(false),
            ..Default::default()
        };
        assert!(!p.previews_enabled());
        p.link_previews = Some(true);
        assert!(
            !p.previews_enabled(),
            "hyperlinks off must override an explicit link_previews: true"
        );
    }

    #[test]
    fn previews_enabled_hyperlinks_on_absent_link_previews_defaults_true() {
        let p = ChatContentPolicy {
            hyperlinks: Some(true),
            link_previews: None,
            ..Default::default()
        };
        assert!(p.previews_enabled());
    }

    #[test]
    fn previews_enabled_hyperlinks_on_explicit_false_disables() {
        let p = ChatContentPolicy {
            hyperlinks: Some(true),
            link_previews: Some(false),
            ..Default::default()
        };
        assert!(!p.previews_enabled());
    }

    #[test]
    fn previews_enabled_hyperlinks_on_explicit_true_enables() {
        let p = ChatContentPolicy {
            hyperlinks: Some(true),
            link_previews: Some(true),
            ..Default::default()
        };
        assert!(p.previews_enabled());
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
        // `markdown` is a type mismatch (string, not bool), so
        // `serde_json::from_value::<ChatContentPolicy>` errors even with
        // `#[serde(default)]` — a merely-missing field would NOT error.
        // Seeded via `seed_document_unvalidated` (raw insert, bypasses
        // ingress validation) since both `apply_intent` and `apply_command`
        // would now reject this Create outright.
        let doc = settings_doc(
            world_id,
            gm,
            serde_json::json!({ "markdown": "not-a-bool" }),
        );
        seed_settings_doc(&repo, world_id, gm, doc).await;
        assert_eq!(
            resolve_content_policy(&repo, world_id).await,
            ChatContentPolicy::default()
        );
    }

    #[tokio::test]
    async fn explicit_null_link_previews_deserializes_to_none() {
        // The GM tri-state "Default" option writes a literal JSON `null` for
        // `link_previews` (not an absent key). It MUST resolve to `None`, so
        // `previews_enabled` follows the default-on-when-hyperlinks rule — a
        // parse failure here would fail-close the WHOLE policy to all-off.
        let (repo, world_id, gm) = world().await;
        let doc = settings_doc(
            world_id,
            gm,
            serde_json::json!({ "hyperlinks": true, "link_previews": null }),
        );
        seed_settings_doc(&repo, world_id, gm, doc).await;
        let p = resolve_content_policy(&repo, world_id).await;
        assert_eq!(p.link_previews, None);
        assert!(p.hyperlinks() && p.previews_enabled());
    }

    #[tokio::test]
    async fn duplicate_settings_docs_resolve_deterministically_by_lowest_id() {
        // No construction-time uniqueness guard yet (see the resolver doc +
        // TODO.md); resolution must still be DETERMINISTIC — `query_documents`
        // orders by id, so the lowest-UUID doc's policy always wins.
        let (repo, world_id, gm) = world().await;
        let mut low = settings_doc(world_id, gm, serde_json::json!({ "markdown": true }));
        low.id = Uuid::from_u128(1);
        let mut high = settings_doc(
            world_id,
            gm,
            serde_json::json!({ "markdown": false, "html": true }),
        );
        high.id = Uuid::from_u128(u128::MAX);
        // Insert the high-id doc FIRST to prove insertion order doesn't decide it.
        seed_settings_doc(&repo, world_id, gm, high).await;
        seed_settings_doc(&repo, world_id, gm, low).await;
        let p = resolve_content_policy(&repo, world_id).await;
        assert!(
            p.markdown() && !p.html(),
            "the lowest-id doc's policy must win deterministically"
        );
    }

    #[tokio::test]
    async fn present_policy_is_read() {
        let (repo, world_id, gm) = world().await;
        let doc = settings_doc(
            world_id,
            gm,
            serde_json::json!({
                "markdown": true, "html": false, "images": true,
                "hyperlinks": true, "emails": false
            }),
        );
        seed_settings_doc(&repo, world_id, gm, doc).await;
        let p = resolve_content_policy(&repo, world_id).await;
        assert!(p.markdown() && p.images() && p.hyperlinks() && !p.html() && !p.emails());
    }

    /// A `dice-settings` `Document` in `world_id` with the given `engine`
    /// body, owned by `gm`.
    fn dice_settings_doc(world_id: Uuid, gm: Uuid, engine: serde_json::Value) -> Document {
        Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id },
            doc_type: DICE_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: Some(gm),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            engine: Some(engine),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
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
        // deserialization into `DiceSettingsEngine` errors outright. Seeded
        // via `seed_document_unvalidated` (raw insert) — both `apply_intent`
        // and `apply_command` would reject this Create.
        let doc = dice_settings_doc(world_id, gm, serde_json::json!({ "mode": 5 }));
        seed_settings_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::Total);
        assert_eq!(ctx.direction, Direction::HighWins);
    }

    #[tokio::test]
    async fn unknown_enum_variant_string_resolves_to_default() {
        let (repo, world_id, gm) = world().await;
        // An out-of-vocabulary variant string (not a type mismatch) also fails
        // the whole-body deserialization — no #[serde(other)] catch-all exists,
        // so fail-closed covers this distinct failure class too. Seeded via
        // `seed_document_unvalidated` for the same ingress-bypass reason as above.
        let doc = dice_settings_doc(
            world_id,
            gm,
            serde_json::json!({ "mode": "foobar", "direction": "low_wins" }),
        );
        seed_settings_doc(&repo, world_id, gm, doc).await;
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
        seed_settings_doc(&repo, world_id, gm, doc).await;
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
        seed_settings_doc(&repo, world_id, gm, doc).await;
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
        seed_settings_doc(&repo, world_id, gm, doc).await;
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
        seed_settings_doc(&repo, world_id, gm, doc).await;
        let ctx = resolve_dice_context(&repo, world_id).await;
        assert_eq!(ctx.mode, ModeKind::SuccessCount);
        assert_eq!(ctx.direction, Direction::LowWins);
    }

    #[tokio::test]
    async fn partial_body_defaults_the_other_field() {
        let (repo, world_id, gm) = world().await;
        // Only `mode` set; `direction` must default to `HighWins`.
        let doc = dice_settings_doc(world_id, gm, serde_json::json!({ "mode": "success_count" }));
        seed_settings_doc(&repo, world_id, gm, doc).await;
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
        seed_settings_doc(&repo2, world_id2, gm2, doc2).await;
        let ctx2 = resolve_dice_context(&repo2, world_id2).await;
        assert_eq!(ctx2.mode, ModeKind::Total);
        assert_eq!(ctx2.direction, Direction::LowWins);
    }
}
