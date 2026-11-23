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
    assert_eq!(drawn, Some(image(member)));
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
    assert_eq!(drawn, Some(image(member)));
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
    assert_eq!(embedded.visual, image(member));
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

    assert_eq!(stored_token(&r, raw.id).await.visual, Some(image(fallback)));
    for id in [linked.id, overridden.id] {
        let drawn = stored_token(&r, id).await.overrides.and_then(|o| o.visual);
        assert_eq!(drawn, Some(image(fallback)));
    }
    let stored = r.get_document(instanced.id).await.unwrap().unwrap();
    let embedded: ActorEngine =
        serde_json::from_value(stored.embedded["actor"][0].engine.clone().unwrap()).unwrap();
    assert_eq!(embedded.visual, image(fallback));
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
        Some(image(fallback))
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
