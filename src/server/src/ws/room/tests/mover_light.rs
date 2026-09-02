//! `Room::execute_move`'s carried-light timeline (`MoveStream.mover_light`): present exactly
//! when the mover carries an enabled emission in an environment-lit scene, sampled at the
//! position samples' own instants, and absent — with no raycast — otherwise.

use super::*;
use serde_json::json;

/// Publish a torch-carrying actor and link the harness token to it (a GM write: carried
/// emissions edit the shared illumination field, so only a GM may author one).
async fn link_torch(h: &MovementHandle, enabled: bool) {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let actor_id = Uuid::from_u128(0x70AC);
    let mut actor = wdoc(h.world_id, actor_id, "actor");
    actor.owner = Some(h.gm.user_id);
    actor.engine = Some(json!({
        "displayName": "Torch Bearer",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
        "light": { "color": "#ffcc66", "intensity": 1.0, "brightRadius": 2.0, "dimRadius": 4.0,
                   "enabled": enabled },
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![FieldChange {
                    path: "/engine/actor_id".into(),
                    old: json!(null),
                    new: json!(actor_id.to_string()),
                    remove: false,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

/// Execute the harness's one-cell move as `who` and return the frame's position samples and
/// light timeline.
async fn light_timeline(
    h: &MovementHandle,
    who: &PermissionContext,
    req: u128,
) -> (
    Vec<crate::ws::protocol::PosSample>,
    Option<Vec<crate::ws::protocol::LightSample>>,
) {
    let res = h
        .room
        .execute_move(
            &h.repo,
            who,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::from_u128(req),
            },
        )
        .await
        .unwrap();
    let ServerMsg::MoveStream {
        samples,
        mover_light,
        ..
    } = res.frame.as_ref()
    else {
        panic!("frame must be a MoveStream");
    };
    (samples.clone(), mover_light.clone())
}

#[tokio::test]
async fn lightless_mover_gets_no_light_timeline() {
    let h = movement_scene("unrestricted", false).await;
    let (_, light) = light_timeline(&h, &h.player, 0x11).await;
    assert!(
        light.is_none(),
        "a token carrying no emission samples no light"
    );
}

#[tokio::test]
async fn carried_torch_is_sampled_at_every_position_instant() {
    // Each light sample pairs with a position sample by `t_ms` and sits AT that sample's
    // position; the authored cell radii reach the wire in scene units (grid size 100).
    let h = movement_scene("unrestricted", false).await;
    link_torch(&h, true).await;
    let (samples, light) = light_timeline(&h, &h.player, 0x12).await;
    let light = light.expect("a carried torch in an environment-lit scene is sampled");
    assert_eq!(light.len(), samples.len());
    for (l, s) in light.iter().zip(&samples) {
        assert_eq!(l.t_ms, s.t_ms);
        assert_eq!(l.pos, s.pos, "the glow travels with the token");
        assert_eq!((l.bright, l.dim), (200.0, 400.0));
        assert_eq!(l.color, 0xFFCC66);
        assert_eq!(
            (l.intensity, l.falloff),
            (1.0, crate::data::engine::FalloffCurve::Linear),
            "the sample is self-describing: an unauthored curve is the linear read default"
        );
        assert!(
            l.polygons.iter().any(|p| p.len() >= 3),
            "each sample carries a raycast illumination polygon"
        );
    }
}

#[tokio::test]
async fn a_gm_mover_lights_the_corridor_too() {
    // The light timeline is not gated on the mover's role: a GM walking a torch-bearing NPC
    // is exactly the case an observing player must see lit mid-walk.
    let h = movement_scene("unrestricted", false).await;
    link_torch(&h, true).await;
    let (samples, light) = light_timeline(&h, &h.gm, 0x13).await;
    assert_eq!(light.map(|l| l.len()), Some(samples.len()));
}

#[tokio::test]
async fn a_suppressed_emission_samples_nothing() {
    let h = movement_scene("unrestricted", false).await;
    link_torch(&h, false).await;
    let (_, light) = light_timeline(&h, &h.player, 0x14).await;
    assert!(light.is_none(), "`enabled: false` is the suppress path");
}

#[tokio::test]
async fn an_all_bright_scene_samples_nothing() {
    // Global illumination has no light field to sample into (`ResolvedScene::all_bright`).
    let h = movement_scene("unrestricted", false).await;
    link_torch(&h, true).await;
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(0x5CE2),
                changes: vec![FieldChange {
                    path: "/engine/scene/lightMode".into(),
                    old: json!("environmentLight"),
                    new: json!("globalIllumination"),
                    remove: false,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let (_, light) = light_timeline(&h, &h.player, 0x15).await;
    assert!(light.is_none());
}
