//! Create-time draw of `TokenVisual::RandomVariant` pools (`resolve_token_random_variants`).

use super::*;
use crate::data::asset::{Asset, AssetMeta};
use crate::data::engine::{
    ActorEngine, Size, TokenArtFilter, TokenEngine, TokenOverrides, TokenVisual,
};

/// An image asset in `world`, optionally filed in `folder` and carrying explicit `tags`.
async fn seed_asset(
    r: &SqliteRepository,
    world: Uuid,
    folder: Option<Uuid>,
    tags: &[&str],
) -> Uuid {
    let id = Uuid::new_v4();
    let asset = Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "art.png".into(),
        content_type: "image/webp".into(),
        byte_size: 10,
        created_by: None,
        created_at: 1,
        version: 1,
        folder_id: folder,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta::unprocessed("image/png", 10),
    };
    r.insert_asset(&asset).await.unwrap();
    let explicit: Vec<String> = tags.iter().map(|t| (*t).to_string()).collect();
    r.set_asset_tags(id, &explicit, &[]).await.unwrap();
    id
}

/// A concrete image visual naming `asset`.
fn image(asset: Uuid) -> TokenVisual {
    TokenVisual::Image {
        asset: asset.to_string(),
        transform: None,
        drawn: false,
    }
}

/// A concrete image visual the server drew for `asset`.
fn drawn_image(asset: Uuid) -> TokenVisual {
    TokenVisual::Image {
        asset: asset.to_string(),
        transform: None,
        drawn: true,
    }
}

/// A pool over `folder` and `tags` whose fallback is `fallback`.
fn pool(folder: Option<Uuid>, tags: &[&str], fallback: Uuid) -> TokenVisual {
    TokenVisual::RandomVariant {
        filter: TokenArtFilter {
            folder,
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
        },
        fallback: Box::new(image(fallback)),
    }
}

/// The engine body of an actor whose visual is `visual`.
fn actor_engine(visual: TokenVisual) -> serde_json::Value {
    serde_json::to_value(ActorEngine {
        display_name: "Actor".into(),
        visual,
        size: Size { w: 1.0, h: 1.0 },
        shape: "square".into(),
        faction: None,
        conditions: vec![],
        prototype: false,
        vision: None,
        facing_vision: None,
        light: None,
        movement: vec![],
        aura: None,
        sound: None,
        vfx: None,
        bars: None,
        nameplate_mode: None,
    })
    .unwrap()
}

/// A world actor document with the given visual.
fn actor_with(world: Uuid, visual: TokenVisual) -> Document {
    let mut d = actor_doc_owned_by(world, None);
    d.engine = Some(actor_engine(visual));
    d
}

/// Creates `doc` as the GM.
async fn create(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    docs: Vec<Document>,
) -> Result<(), DataError> {
    r.apply_intent(
        ctx,
        w,
        docs.into_iter()
            .map(|doc| Operation::Create { doc })
            .collect(),
        1,
        WriteOrigin::Client,
    )
    .await
    .map(|_| ())
}

/// The stored token engine of `id`.
async fn stored_token(r: &SqliteRepository, id: Uuid) -> TokenEngine {
    let doc = r.get_document(id).await.unwrap().unwrap();
    serde_json::from_value(doc.engine.unwrap()).unwrap()
}

/// The stored actor engine body of `id`.
async fn stored_actor(r: &SqliteRepository, id: Uuid) -> ActorEngine {
    let doc = r.get_document(id).await.unwrap().unwrap();
    serde_json::from_value(doc.engine.unwrap()).unwrap()
}

/// A raw token whose own `visual` is `visual`.
fn raw_token(world: Uuid, visual: TokenVisual) -> Document {
    let mut t = owned_token_doc(world, None);
    t.engine.as_mut().unwrap()["visual"] = serde_json::to_value(visual).unwrap();
    t
}

/// A token linked to `actor`, with `overrides_visual` set when given.
fn linked_token(world: Uuid, actor: Uuid, overrides_visual: Option<TokenVisual>) -> Document {
    let mut t = owned_token_doc(world, Some(actor));
    if let Some(v) = overrides_visual {
        let overrides = TokenOverrides {
            name: None,
            visual: Some(v),
            size: None,
            shape: None,
            vision: None,
            facing_vision: None,
            light: None,
            movement: None,
            aura: None,
            sound: None,
            vfx: None,
            disposition: None,
            bars: None,
            nameplate_mode: None,
        };
        t.engine.as_mut().unwrap()["overrides"] = serde_json::to_value(overrides).unwrap();
    }
    t
}

#[tokio::test]
async fn a_raw_token_pool_is_stored_as_one_of_the_matching_assets() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let mut members = Vec::new();
    for _ in 0..3 {
        members.push(seed_asset(&r, w, None, &["goblin"]).await);
    }
    let token = raw_token(w, pool(None, &["goblin"], fallback));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
    let stored = stored_token(&r, token.id).await;
    match stored.visual {
        Some(TokenVisual::Image { asset, .. }) => {
            let drawn = Uuid::parse_str(&asset).unwrap();
            assert!(members.contains(&drawn), "drew a non-member: {drawn}");
            assert_ne!(
                drawn, fallback,
                "the fallback must not be drawn from a non-empty pool"
            );
        }
        other => panic!("a stored token must hold a concrete image, got {other:?}"),
    }
}

#[tokio::test]
async fn draws_cover_every_pool_member_over_many_creates() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let a = seed_asset(&r, w, None, &["pair"]).await;
    let b = seed_asset(&r, w, None, &["pair"]).await;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        let token = raw_token(w, pool(None, &["pair"], fallback));
        create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
        if let Some(TokenVisual::Image { asset, .. }) = stored_token(&r, token.id).await.visual {
            seen.insert(Uuid::parse_str(&asset).unwrap());
        }
    }
    assert_eq!(seen, std::collections::HashSet::from([a, b]));
}

#[tokio::test]
async fn a_linked_token_draws_into_its_own_overrides_and_leaves_the_actor_a_pool() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["orc"]).await;
    let actor = actor_with(w, pool(None, &["orc"], fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let token = linked_token(w, actor.id, None);
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();

    let stored = stored_token(&r, token.id).await;
    let drawn = stored.overrides.and_then(|o| o.visual);
    assert_eq!(drawn, Some(drawn_image(member)));
    assert!(
        matches!(
            stored_actor(&r, actor.id).await.visual,
            TokenVisual::RandomVariant { .. }
        ),
        "the shared actor's own pool must persist untouched"
    );
}

#[tokio::test]
async fn a_concrete_overrides_visual_suppresses_the_linked_actor_draw() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let _member = seed_asset(&r, w, None, &["orc"]).await;
    let chosen = seed_asset(&r, w, None, &[]).await;
    let actor = actor_with(w, pool(None, &["orc"], fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let token = linked_token(w, actor.id, Some(image(chosen)));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
    let drawn = stored_token(&r, token.id)
        .await
        .overrides
        .and_then(|o| o.visual);
    assert_eq!(drawn, Some(image(chosen)));
}

#[tokio::test]
async fn an_overrides_pool_is_drawn_in_place() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["elf"]).await;
    let actor = actor_with(w, image(fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let token = linked_token(w, actor.id, Some(pool(None, &["elf"], fallback)));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
    let drawn = stored_token(&r, token.id)
        .await
        .overrides
        .and_then(|o| o.visual);
    assert_eq!(drawn, Some(drawn_image(member)));
}

#[tokio::test]
async fn an_instanced_embedded_actor_copy_is_drawn_and_the_token_visual_stays_none() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["imp"]).await;
    let mut token = owned_token_doc(w, None);
    let copy = actor_with(w, pool(None, &["imp"], fallback));
    token.embedded.insert("actor".into(), vec![copy]);
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();

    let stored = r.get_document(token.id).await.unwrap().unwrap();
    let embedded: ActorEngine =
        serde_json::from_value(stored.embedded["actor"][0].engine.clone().unwrap()).unwrap();
    assert_eq!(embedded.visual, drawn_image(member));
    assert!(stored_token(&r, token.id).await.visual.is_none());
}

#[tokio::test]
async fn an_empty_pool_yields_the_fallback_at_every_draw_site() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let empty = pool(None, &["no-such-tag"], fallback);

    let raw = raw_token(w, empty.clone());
    let actor = actor_with(w, empty.clone());
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let linked = linked_token(w, actor.id, None);
    let overridden = linked_token(w, actor.id, Some(empty.clone()));
    let mut instanced = owned_token_doc(w, None);
    instanced
        .embedded
        .insert("actor".into(), vec![actor_with(w, empty)]);
    create(
        &r,
        &ctx,
        w,
        vec![
            raw.clone(),
            linked.clone(),
            overridden.clone(),
            instanced.clone(),
        ],
    )
    .await
    .unwrap();

    assert_eq!(
        stored_token(&r, raw.id).await.visual,
        Some(drawn_image(fallback))
    );
    for id in [linked.id, overridden.id] {
        let drawn = stored_token(&r, id).await.overrides.and_then(|o| o.visual);
        assert_eq!(drawn, Some(drawn_image(fallback)));
    }
    let stored = r.get_document(instanced.id).await.unwrap().unwrap();
    let embedded: ActorEngine =
        serde_json::from_value(stored.embedded["actor"][0].engine.clone().unwrap()).unwrap();
    assert_eq!(embedded.visual, drawn_image(fallback));
}

#[tokio::test]
async fn a_folder_pool_draws_only_from_that_subtree() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let top = folder_doc(1, w, "top", None);
    let sub = folder_doc(2, w, "sub", Some(top.id));
    let other = folder_doc(3, w, "other", None);
    create(&r, &ctx, w, vec![top.clone(), sub.clone(), other.clone()])
        .await
        .unwrap();
    let fallback = seed_asset(&r, w, None, &[]).await;
    let in_top = seed_asset(&r, w, Some(top.id), &[]).await;
    let in_sub = seed_asset(&r, w, Some(sub.id), &[]).await;
    for _ in 0..4 {
        seed_asset(&r, w, Some(other.id), &[]).await;
        seed_asset(&r, w, None, &[]).await;
    }
    for _ in 0..30 {
        let token = raw_token(w, pool(Some(top.id), &[], fallback));
        create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
        let Some(TokenVisual::Image { asset, .. }) = stored_token(&r, token.id).await.visual else {
            panic!("expected a concrete image");
        };
        let drawn = Uuid::parse_str(&asset).unwrap();
        assert!(
            drawn == in_top || drawn == in_sub,
            "drew {drawn} from outside the folder subtree"
        );
    }
}

#[tokio::test]
async fn a_pool_is_scoped_to_the_tokens_world() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let w2 = r.create_world("other", 1).await.unwrap().id;
    let fallback = seed_asset(&r, w, None, &[]).await;
    seed_asset(&r, w2, None, &["shared"]).await;
    let token = raw_token(w, pool(None, &["shared"], fallback));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
    assert_eq!(
        stored_token(&r, token.id).await.visual,
        Some(drawn_image(fallback))
    );
}

#[tokio::test]
async fn a_pool_fallback_that_is_a_pool_refuses_the_create() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let nested = TokenVisual::RandomVariant {
        filter: TokenArtFilter::default(),
        fallback: Box::new(pool(None, &[], fallback)),
    };
    let token = raw_token(w, nested);
    let err = create(&r, &ctx, w, vec![token.clone()]).await.unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)), "got {err:?}");
    assert!(r.get_document(token.id).await.unwrap().is_none());
}

/// Applies one `Update` as the GM and returns the stored command.
async fn update(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> StoredCommand {
    r.apply_intent(
        ctx,
        w,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: path.into(),
                old,
                new,
            }],
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_pool_set_on_a_placed_token_by_an_update_is_drawn_and_broadcast_drawn() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["troll"]).await;
    let token = raw_token(w, image(fallback));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();

    let stored = update(
        &r,
        &ctx,
        w,
        token.id,
        "/engine/visual",
        serde_json::to_value(image(fallback)).unwrap(),
        serde_json::to_value(pool(None, &["troll"], fallback)).unwrap(),
    )
    .await;

    assert_eq!(
        stored_token(&r, token.id).await.visual,
        Some(drawn_image(member))
    );
    let Operation::Update { changes, .. } = &stored.command.ops[0] else {
        panic!("expected the token Update");
    };
    assert_eq!(
        changes[0].new,
        serde_json::to_value(drawn_image(member)).unwrap(),
        "the broadcast carries the drawn value, never the pool"
    );
}

#[tokio::test]
async fn saving_an_actors_pool_redraws_every_token_already_linked_to_it() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["wolf"]).await;
    let actor = actor_with(w, image(fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let bare = linked_token(w, actor.id, None);
    let other = actor_with(w, image(fallback));
    create(&r, &ctx, w, vec![other.clone()]).await.unwrap();
    let unrelated = linked_token(w, other.id, None);
    create(&r, &ctx, w, vec![bare.clone(), unrelated.clone()])
        .await
        .unwrap();

    let stored = update(
        &r,
        &ctx,
        w,
        actor.id,
        "/engine/visual",
        serde_json::to_value(image(fallback)).unwrap(),
        serde_json::to_value(pool(None, &["wolf"], fallback)).unwrap(),
    )
    .await;

    let drawn = stored_token(&r, bare.id)
        .await
        .overrides
        .and_then(|o| o.visual);
    assert_eq!(
        drawn,
        Some(drawn_image(member)),
        "the linked token must be redrawn"
    );
    assert!(
        stored_token(&r, unrelated.id)
            .await
            .overrides
            .and_then(|o| o.visual)
            .is_none(),
        "a token linked to another actor is untouched"
    );
    assert_eq!(
        stored.command.ops.len(),
        2,
        "the actor Update plus one Update per linked token"
    );
}

/// An actor with a one-member pool and a token linked to it, the token's art drawn at Create.
/// Returns (world, GM context, actor id, token id, the pool member, the fallback).
async fn pool_actor_with_drawn_token() -> (
    SqliteRepository,
    Uuid,
    crate::data::membership::PermissionContext,
    Uuid,
    Uuid,
    Uuid,
    Uuid,
) {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let member = seed_asset(&r, w, None, &["bat"]).await;
    let actor = actor_with(w, pool(None, &["bat"], fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let token = linked_token(w, actor.id, None);
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();
    assert_eq!(
        stored_token(&r, token.id)
            .await
            .overrides
            .and_then(|o| o.visual),
        Some(drawn_image(member)),
        "the Create draws once"
    );
    (r, w, ctx, actor.id, token.id, member, fallback)
}

/// Saves the actor's pool again with fallback `to` in place of `from`, which draws for linked
/// tokens.
async fn resave_pool(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    actor: Uuid,
    from: Uuid,
    to: Uuid,
) {
    update(
        r,
        ctx,
        w,
        actor,
        "/engine/visual",
        serde_json::to_value(pool(None, &["bat"], from)).unwrap(),
        serde_json::to_value(pool(None, &["bat"], to)).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn clearing_a_drawn_override_on_a_pool_actor_draws_again_instead_of_leaving_no_art() {
    let (r, w, ctx, _actor, token, member, _fallback) = pool_actor_with_drawn_token().await;

    let stored = update(
        &r,
        &ctx,
        w,
        token,
        "/engine/overrides/visual",
        serde_json::to_value(drawn_image(member)).unwrap(),
        serde_json::Value::Null,
    )
    .await;

    assert_eq!(
        stored_token(&r, token)
            .await
            .overrides
            .and_then(|o| o.visual),
        Some(drawn_image(member)),
        "a Clear leaves the token with the pool's pick, never with no art"
    );
    let Operation::Update { changes, .. } = &stored.command.ops[0] else {
        panic!("expected the token Update");
    };
    assert_eq!(
        changes[0].new,
        serde_json::to_value(drawn_image(member)).unwrap(),
        "the broadcast carries the pick, not the null the client sent"
    );
}

#[tokio::test]
async fn an_actor_pool_save_keeps_art_a_gm_set_by_hand_on_a_linked_token() {
    let (r, w, ctx, actor, token, member, fallback) = pool_actor_with_drawn_token().await;
    let chosen = seed_asset(&r, w, None, &[]).await;
    update(
        &r,
        &ctx,
        w,
        token,
        "/engine/overrides/visual",
        serde_json::to_value(drawn_image(member)).unwrap(),
        serde_json::to_value(image(chosen)).unwrap(),
    )
    .await;

    let other_fallback = seed_asset(&r, w, None, &[]).await;
    resave_pool(&r, &ctx, w, actor, fallback, other_fallback).await;

    assert_eq!(
        stored_token(&r, token)
            .await
            .overrides
            .and_then(|o| o.visual),
        Some(image(chosen)),
        "hand-set art survives a pool save"
    );
}

#[tokio::test]
async fn an_actor_pool_save_keeps_art_a_create_carried_by_hand() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let fallback = seed_asset(&r, w, None, &[]).await;
    let _member = seed_asset(&r, w, None, &["bat"]).await;
    let chosen = seed_asset(&r, w, None, &[]).await;
    let actor = actor_with(w, pool(None, &["bat"], fallback));
    create(&r, &ctx, w, vec![actor.clone()]).await.unwrap();
    let token = linked_token(w, actor.id, Some(image(chosen)));
    create(&r, &ctx, w, vec![token.clone()]).await.unwrap();

    let other_fallback = seed_asset(&r, w, None, &[]).await;
    resave_pool(&r, &ctx, w, actor.id, fallback, other_fallback).await;

    assert_eq!(
        stored_token(&r, token.id)
            .await
            .overrides
            .and_then(|o| o.visual),
        Some(image(chosen))
    );
}

#[tokio::test]
async fn an_actor_pool_save_redraws_art_the_server_drew_and_keeps_it_redrawable() {
    let (r, w, ctx, actor, token, member, fallback) = pool_actor_with_drawn_token().await;
    let later = seed_asset(&r, w, None, &["bat"]).await;
    let spare = seed_asset(&r, w, None, &[]).await;
    let mut seen: Vec<TokenVisual> = Vec::new();
    let (mut from, mut to) = (fallback, spare);
    for _ in 0..40 {
        resave_pool(&r, &ctx, w, actor, from, to).await;
        (from, to) = (to, from);
        let drawn = stored_token(&r, token)
            .await
            .overrides
            .and_then(|o| o.visual);
        let drawn = drawn.unwrap();
        if !seen.contains(&drawn) {
            seen.push(drawn);
        }
        if seen.len() == 2 {
            break;
        }
    }
    assert!(
        seen.contains(&drawn_image(member)) && seen.contains(&drawn_image(later)),
        "server-drawn art is redrawn on every save, got {seen:?}"
    );
}

#[tokio::test]
async fn editing_another_override_keeps_server_drawn_art_redrawable() {
    let (r, w, ctx, actor, token, member, fallback) = pool_actor_with_drawn_token().await;
    update(
        &r,
        &ctx,
        w,
        token,
        "/engine/overrides",
        serde_json::json!({ "visual": drawn_image(member) }),
        serde_json::json!({ "visual": drawn_image(member), "name": "Renamed" }),
    )
    .await;
    let later = seed_asset(&r, w, None, &["bat"]).await;
    let spare = seed_asset(&r, w, None, &[]).await;
    let mut redrawn = false;
    let (mut from, mut to) = (fallback, spare);
    for _ in 0..40 {
        resave_pool(&r, &ctx, w, actor, from, to).await;
        (from, to) = (to, from);
        if stored_token(&r, token)
            .await
            .overrides
            .and_then(|o| o.visual)
            == Some(drawn_image(later))
        {
            redrawn = true;
            break;
        }
    }
    assert!(
        redrawn,
        "an unrelated override edit must not freeze the drawn art"
    );
}
