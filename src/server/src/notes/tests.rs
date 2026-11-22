//! These tests cover `handle_present_note`: the GM gate, the share requirement, world scoping,
//! the shared rate bucket, and the broadcast frame.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::{Document, PermissionSet, Scope};
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::{RoomEvent, RoomRegistry};

/// A world with a GM, a player, and a room.
struct Fixture {
    /// The repository behind everything.
    repo: SqliteRepository,
    /// The world's room.
    room: std::sync::Arc<Room>,
    /// The world's id.
    world: Uuid,
    /// The GM's user id.
    gm: Uuid,
    /// The player's user id.
    player: Uuid,
}

impl Fixture {
    /// Builds the world with its GM and one player member.
    async fn new() -> Self {
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
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        Fixture {
            repo,
            room,
            world: w.id,
            gm,
            player,
        }
    }

    /// Seeds a document of `doc_type` in `world` whose default role is `default`.
    async fn seed(&self, world: Uuid, doc_type: &str, default: DocRole) -> Uuid {
        let id = Uuid::new_v4();
        self.repo
            .seed_document_unvalidated(&Document {
                id,
                scope: Scope::World { world_id: world },
                doc_type: doc_type.into(),
                schema_version: 1,
                name: Some("Handout".into()),
                source: None,
                base: None,
                owner: None,
                permissions: PermissionSet {
                    default,
                    users: Default::default(),
                    property_overrides: Default::default(),
                    capabilities: Default::default(),
                    gm_role: None,
                },
                embedded: Default::default(),
                parent_id: None,
                engine: None,
                system: serde_json::json!({}),
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
        id
    }

    /// Runs the handler as `user` with `role` against a fresh rate bucket.
    async fn present(
        &self,
        user: Uuid,
        role: WorldRole,
        note: Uuid,
    ) -> Result<(), PresentNoteError> {
        let ctx = PermissionContext {
            user_id: user,
            world_role: role,
        };
        handle_present_note(
            &self.room,
            &self.repo,
            &ctx,
            &PingRateLimiter::new(),
            note,
            0,
        )
        .await
    }
}

#[tokio::test]
async fn a_shared_note_broadcasts_present_note_stamped_with_the_gm() {
    let f = Fixture::new().await;
    let note = f.seed(f.world, NOTE_DOC_TYPE, DocRole::Observer).await;
    let (mut rx, _current) = f.room.subscribe();
    f.present(f.gm, WorldRole::Gm, note).await.unwrap();
    let RoomEvent::Other(msg) = rx.recv().await.unwrap() else {
        panic!("expected an out-of-band frame");
    };
    match &*msg {
        ServerMsg::PresentNote {
            note_id,
            presented_by,
        } => {
            assert_eq!(*note_id, note);
            assert_eq!(*presented_by, f.gm);
        }
        other => panic!("expected ServerMsg::PresentNote, got {other:?}"),
    }
}

#[tokio::test]
async fn a_private_note_is_refused_without_a_broadcast() {
    let f = Fixture::new().await;
    let note = f.seed(f.world, NOTE_DOC_TYPE, DocRole::None).await;
    let (mut rx, _current) = f.room.subscribe();
    let r = f.present(f.gm, WorldRole::Gm, note).await;
    assert!(matches!(r, Err(PresentNoteError::NotShared)));
    assert!(rx.try_recv().is_err(), "a refusal broadcasts nothing");
}

#[tokio::test]
async fn a_player_is_refused_even_for_a_shared_note() {
    let f = Fixture::new().await;
    let note = f.seed(f.world, NOTE_DOC_TYPE, DocRole::Observer).await;
    let (mut rx, _current) = f.room.subscribe();
    let r = f.present(f.player, WorldRole::Player, note).await;
    assert!(matches!(r, Err(PresentNoteError::Forbidden)));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn a_non_note_and_a_foreign_world_note_are_not_found() {
    let f = Fixture::new().await;
    let scene = f.seed(f.world, "scene", DocRole::Observer).await;
    let other_world = f.repo.create_world_owned("Other", f.gm, 0).await.unwrap();
    let foreign = f
        .seed(other_world.id, NOTE_DOC_TYPE, DocRole::Observer)
        .await;
    for id in [scene, foreign, Uuid::new_v4()] {
        let r = f.present(f.gm, WorldRole::Gm, id).await;
        assert!(matches!(r, Err(PresentNoteError::NotFound)), "{r:?}");
    }
}

#[tokio::test]
async fn presenting_spends_the_shared_vfx_bucket() {
    let f = Fixture::new().await;
    let note = f.seed(f.world, NOTE_DOC_TYPE, DocRole::Observer).await;
    let ctx = PermissionContext {
        user_id: f.gm,
        world_role: WorldRole::Gm,
    };
    let rate = PingRateLimiter::new();
    for _ in 0..crate::ws::VFX_RATE_PER_MIN {
        assert!(rate.check(f.gm, 0, crate::ws::VFX_RATE_PER_MIN));
    }
    // The bucket is drained by other front doors (`/fx`, `PlayVfx`): presenting is refused.
    let r = handle_present_note(&f.room, &f.repo, &ctx, &rate, note, 0).await;
    assert!(matches!(r, Err(PresentNoteError::RateLimited)));
}
