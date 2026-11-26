//! Tests for the two blind-roll settings: which users hold a blind message under each setting
//! combination, on every path a reader gets a message from (broadcast, resync, list, search),
//! and that creation and revision derive one readership.

use super::revise_audience_tests::{harness, wire_ops, Harness};
use super::*;
use crate::data::command::Operation;
use crate::data::document::{DocRole, Document, PermissionSet, Scope};
use crate::data::membership::PermissionContext;
use crate::data::permission::{cap, resolve_access};
use crate::data::snapshot::StoredCommand;
use crate::ws::PingRateLimiter;

/// Stores the world's `chat-settings` document with the two blind settings.
pub(super) async fn set_blind_settings(h: &Harness, players: bool, actor_owner: bool) {
    let doc = Document {
        id: Uuid::new_v4(),
        scope: Scope::World {
            world_id: h.room.world_id,
        },
        doc_type: settings::CHAT_SETTINGS_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(h.gm.user_id),
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "blind_visible_to_players": players,
            "blind_visible_to_actor_owner": actor_owner,
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    h.repo.seed_document_unvalidated(&doc).await.unwrap();
}

/// Stores an actor owned by `owner` and returns its id.
async fn seed_actor(h: &Harness, owner: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let doc = Document {
        id,
        scope: Scope::World {
            world_id: h.room.world_id,
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: Some("Hero".into()),
        source: None,
        base: None,
        owner: Some(owner),
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({ "displayName": "Hero" })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    h.repo.seed_document_unvalidated(&doc).await.unwrap();
    id
}

/// Sends a blind roll flavored `xylophone` as `who`, attributed to `actor`, and returns its id.
async fn send_blind(h: &Harness, who: &PermissionContext, actor: Option<Uuid>) -> Uuid {
    let rate = PingRateLimiter::new();
    let (cmd, _) = handle_send_message(
        MessageRequestCtx {
            room: &h.room,
            repo: &h.repo,
            ctx: who,
            rate: &rate,
            vfx_rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "general".into(),
        "/blindroll 1d20 | xylophone".into(),
        actor.map(|actor_id| ActorOwnerRef::Actor { actor_id }),
        Audience::Public,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    }
}

/// Whether the ops delivered to `who` hold the message `id`.
async fn delivered(h: &Harness, who: &PermissionContext, ev: &StoredCommand, id: Uuid) -> bool {
    wire_ops(h, who, ev)
        .await
        .iter()
        .any(|op| matches!(op, Operation::Create { doc } if doc.id == id && doc.engine.is_some()))
}

/// The readership of message `id` for `who` on each path: broadcast, resync replay, list and
/// search. The four must agree, so the result is one boolean; a disagreement fails here.
async fn reads(h: &Harness, who: &PermissionContext, id: Uuid) -> bool {
    let events = h.repo.events_since(h.room.world_id, 0).await.unwrap();
    let ev = events
        .iter()
        .find(|ev| {
            ev.command
                .ops
                .iter()
                .any(|op| matches!(op, Operation::Create { doc } if doc.id == id))
        })
        .expect("the message's create event is stored");
    // The broadcast and the resync replay both filter the stored event.
    let wire = delivered(h, who, ev, id).await;

    let doc = h.repo.get_document(id).await.unwrap().unwrap();
    let listed = resolve_access(who.user_id, who.world_role, &doc, doc.owner).has(cap::READ);

    let searched = h
        .repo
        .search(who, h.room.world_id, "xylophone", 10, None, &[])
        .await
        .unwrap()
        .hits
        .iter()
        .any(|hit| hit.document.id == id);

    assert!(
        wire == listed && listed == searched,
        "paths disagree for {:?}: wire {wire}, list {listed}, search {searched}",
        who.user_id
    );
    wire
}

// Fails if `permissions_for_audience` ignores `BlindReaders::everyone`: the default tier stays
// open and a bystander reads a message the setting hides.
#[tokio::test]
async fn a_players_blind_roll_is_hidden_from_other_players_unless_the_setting_is_on() {
    for players in [false, true] {
        for actor_owner in [false, true] {
            let h = harness().await;
            set_blind_settings(&h, players, actor_owner).await;
            let id = send_blind(&h, &h.sender, None).await;

            assert!(reads(&h, &h.sender, id).await, "the roller keeps a receipt");
            assert!(reads(&h, &h.gm, id).await, "the GM always reads");
            assert_eq!(reads(&h, &h.bystander, id).await, players);
            assert_eq!(reads(&h, &h.friend, id).await, players);
        }
    }
}

// Fails if `resolve_blind_readers` lets `blind_visible_to_players` admit readers of a GM's
// blind roll (the bystander and the unrelated player then read it), or drops the actor's owner
// or reads the wrong setting for them.
#[tokio::test]
async fn a_gms_blind_roll_reaches_the_actors_owner_only_under_its_setting() {
    for players in [false, true] {
        for actor_owner in [false, true] {
            let h = harness().await;
            set_blind_settings(&h, players, actor_owner).await;
            let actor = seed_actor(&h, h.friend.user_id).await;
            let id = send_blind(&h, &h.gm, Some(actor)).await;

            assert!(reads(&h, &h.gm, id).await);
            assert_eq!(
                reads(&h, &h.friend, id).await,
                actor_owner,
                "actor owner, players={players} actor_owner={actor_owner}"
            );
            assert!(
                !reads(&h, &h.bystander, id).await,
                "bystander, players={players} actor_owner={actor_owner}"
            );
            assert!(
                !reads(&h, &h.sender, id).await,
                "an unrelated player never reads a GM's blind roll through the players setting"
            );
        }
    }
}

// The default (no settings document) is the closed shape.
#[tokio::test]
async fn with_no_settings_a_blind_roll_is_invisible_to_every_other_player() {
    let h = harness().await;
    let id = send_blind(&h, &h.sender, None).await;
    assert!(reads(&h, &h.sender, id).await);
    assert!(reads(&h, &h.gm, id).await);
    assert!(!reads(&h, &h.friend, id).await);
    assert!(!reads(&h, &h.bystander, id).await);
}

// A player rolling blind never opens the message to the owner of some other actor through the
// actor-owner setting: that setting concerns a GM's roll.
#[tokio::test]
async fn the_actor_owner_setting_does_not_apply_to_a_players_own_blind_roll() {
    let h = harness().await;
    set_blind_settings(&h, false, true).await;
    let id = send_blind(&h, &h.sender, None).await;
    assert!(!reads(&h, &h.friend, id).await);
}

// Revising a roll to `Blind` stores the same readership creation would for that roller and
// attribution, and a Public revision restores the open shape. Fails if
// `handle_revise_audience` and `build_message_doc_with_readers` stop sharing
// `permissions_for_audience` and `message_property_overrides`: any field below then differs.
#[tokio::test]
async fn revising_to_blind_stores_what_creating_blind_stores() {
    for (players, actor_owner) in [(false, false), (true, false), (false, true)] {
        let h = harness().await;
        set_blind_settings(&h, players, actor_owner).await;
        let actor = seed_actor(&h, h.friend.user_id).await;

        let created = send_blind(&h, &h.gm, Some(actor)).await;
        let created = h.repo.get_document(created).await.unwrap().unwrap();

        let rate = PingRateLimiter::new();
        let plain = {
            let rate = PingRateLimiter::new();
            let (cmd, _) = handle_send_message(
                MessageRequestCtx {
                    room: &h.room,
                    repo: &h.repo,
                    ctx: &h.gm,
                    rate: &rate,
                    vfx_rate: &rate,
                    preview: LinkPreviewDeps {
                        client: &link_preview::build_client_allow_loopback(),
                        cache: &LinkPreviewCache::new(),
                        rate: &PreviewRateLimiter::new(),
                    },
                    now: 100,
                    budget_per_min: 30,
                },
                "general".into(),
                "/roll 1d20 | xylophone".into(),
                Some(ActorOwnerRef::Actor { actor_id: actor }),
                Audience::Public,
                None,
            )
            .await
            .unwrap()
            .unwrap();
            match &cmd.ops[0] {
                Operation::Create { doc } => doc.id,
                other => panic!("expected Create, got {other:?}"),
            }
        };
        handle_revise_audience(
            RecalcRollRequestCtx {
                room: &h.room,
                repo: &h.repo,
                ctx: &h.gm,
                rate: &rate,
                now: 200,
                budget_per_min: 30,
            },
            plain,
            Audience::Blind,
        )
        .await
        .unwrap();
        let revised = h.repo.get_document(plain).await.unwrap().unwrap();

        let p = |d: &Document| {
            (
                d.permissions.default,
                d.permissions.gm_role,
                d.permissions.users.clone(),
                d.permissions.property_overrides.clone(),
            )
        };
        let (cd, cg, cu, co) = p(&created);
        let (rd, rg, ru, ro) = p(&revised);
        assert_eq!((cd, cg), (rd, rg), "players={players} owner={actor_owner}");
        assert_eq!(cu, ru, "players={players} owner={actor_owner}");
        assert_eq!(co, ro, "players={players} owner={actor_owner}");
        assert_eq!(cu.get(&h.gm.user_id), Some(&DocRole::Owner));

        let readers = resolve_blind_readers(
            &h.repo,
            h.room.world_id,
            h.gm.user_id,
            Some(&ActorOwnerRef::Actor { actor_id: actor }),
        )
        .await
        .unwrap();
        let engine: MessageEngine =
            serde_json::from_value(revised.engine.clone().unwrap()).unwrap();
        let rebuilt = build_message_doc_with_readers(
            h.room.world_id,
            h.gm.user_id,
            MessageDraft {
                channel: engine.channel.clone(),
                actor_owner: engine.actor_owner.clone(),
                audience: Audience::Blind,
                kind: engine.kind,
                content: engine.content.clone(),
                source: engine.source.clone(),
                system: None,
            },
            &readers,
            0,
        );
        assert_eq!(
            p(&rebuilt),
            p(&revised),
            "players={players} owner={actor_owner}"
        );

        // Revealing returns the open shape.
        handle_revise_audience(
            RecalcRollRequestCtx {
                room: &h.room,
                repo: &h.repo,
                ctx: &h.gm,
                rate: &PingRateLimiter::new(),
                now: 300,
                budget_per_min: 30,
            },
            plain,
            Audience::Public,
        )
        .await
        .unwrap();
        let open = h.repo.get_document(plain).await.unwrap().unwrap();
        assert_eq!(open.permissions.default, DocRole::Observer);
        assert!(reads(&h, &h.bystander, plain).await);
    }
}

// The decider alone: the readers map onto the stored shape.
#[test]
fn permissions_for_audience_maps_blind_readers_onto_the_shape() {
    let author = Uuid::new_v4();
    let reader = Uuid::new_v4();

    let (d, g, u) = permissions_for_audience(author, &Audience::Blind, &BlindReaders::default());
    assert_eq!((d, g), (DocRole::None, None));
    assert_eq!(u.len(), 1);
    assert_eq!(u.get(&author), Some(&DocRole::Owner));

    let open = BlindReaders {
        everyone: true,
        also: vec![reader, author],
    };
    let (d, g, u) = permissions_for_audience(author, &Audience::Blind, &open);
    assert_eq!((d, g), (DocRole::Observer, None));
    assert_eq!(u.get(&reader), Some(&DocRole::Observer));
    assert_eq!(
        u.get(&author),
        Some(&DocRole::Owner),
        "naming the author as a reader must not demote them"
    );

    // Another audience ignores the readers.
    let (d, _, u) = permissions_for_audience(author, &Audience::Public, &open);
    assert_eq!(d, DocRole::Observer);
    assert_eq!(u.len(), 1);
}
