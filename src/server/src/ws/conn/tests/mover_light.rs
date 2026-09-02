//! Per-recipient admission of the carried-light timeline (`MoveStream.mover_light`) in
//! `clip_move_stream` and the own-move re-emit: full for the mover and a plain GM, disc-
//! admitted per sample for an observer or a GM see-as, gone when nothing reaches.

use super::*;
use crate::ws::protocol::{LightSample, PosSample, VisionSample};
use tokio::sync::Semaphore;

/// A light sample at `pos` with dim reach `dim`, identified by `t_ms` alone.
fn lamp(t_ms: f64, pos: [f64; 2], dim: f64) -> LightSample {
    LightSample {
        t_ms,
        pos,
        bright: dim / 2.0,
        dim,
        intensity: 1.0,
        falloff: crate::data::engine::FalloffCurve::Linear,
        color: 0xFFCC66,
        polygons: vec![vec![
            [pos[0] - dim, pos[1] - dim],
            [pos[0] + dim, pos[1] - dim],
            [pos[0] + dim, pos[1] + dim],
            [pos[0] - dim, pos[1] + dim],
        ]],
    }
}

/// A stranger's two-sample walk starting in the observer's view at (50,60) and ending behind
/// the x=100 wall at (250,50), carrying a torch whose reach at the far sample is `far_dim`.
fn torch_walk(scene: Uuid, now: i64, far_dim: f64) -> ServerMsg {
    ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0x71),
        token_id: Uuid::from_u128(0x72),
        mover: Uuid::from_u128(0xAABB),
        scene,
        start_server_ms: now as f64,
        duration_ms: 1_000.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 60.0],
            },
            PosSample {
                t_ms: 1_000.0,
                pos: [250.0, 50.0],
            },
        ],
        mover_vision: None,
        mover_light: Some(vec![
            lamp(0.0, [50.0, 60.0], 50.0),
            lamp(1_000.0, [250.0, 50.0], far_dim),
        ]),
        cost: Some(2.0),
        truncated: Some(false),
    }
}

/// The recipient's clip of `frame`: `(position sample count, light sample t_ms list)`, or
/// `None` when the frame is suppressed.
async fn clip_for(
    frame: &ServerMsg,
    ctx: &PermissionContext,
    see_as: Option<PermissionContext>,
    room: &crate::ws::room::Room,
) -> Option<(usize, Option<Vec<f64>>)> {
    let out = clip_move_stream(
        frame,
        ctx,
        see_as,
        room,
        &crate::data::document::WorldCapDefaults::default(),
    )
    .await?;
    let ServerMsg::MoveStream {
        samples,
        mover_light,
        mover_vision,
        ..
    } = out
    else {
        panic!("clip returns a MoveStream")
    };
    if ctx.user_id != Uuid::from_u128(0xAABB) {
        assert!(mover_vision.is_none(), "mover_vision stays mover-only");
    }
    Some((
        samples.len(),
        mover_light.map(|l| l.iter().map(|s| s.t_ms).collect()),
    ))
}

/// The observer's-side sight wall at x=100 (`setup_clip_room`'s wall body).
fn wall() -> serde_json::Value {
    json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true })
}

#[tokio::test]
async fn observer_admits_a_light_sample_whose_glow_reaches_past_its_clipped_position_prefix() {
    // Position clip: only t=0 is in the observer's view (t=1000 is behind the wall). Light
    // admission: the far sample's 210-unit disc from (250,50) crosses the wall line at x=100
    // (150 away) and lights the observer's own cell center (50,50) inside its vision, so the
    // glow is admitted where the token is not.
    let (room, _, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    let (n, light) = clip_for(&torch_walk(scene_id, now, 210.0), &obs_ctx, None, &room)
        .await
        .expect("the near sample is visible");
    assert_eq!(n, 1, "the position prefix is clipped exactly as before");
    assert_eq!(light, Some(vec![0.0, 1_000.0]));
}

#[tokio::test]
async fn observer_drops_a_light_sample_out_of_reach() {
    let (room, _, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    let (n, light) = clip_for(&torch_walk(scene_id, now, 50.0), &obs_ctx, None, &room)
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        light,
        Some(vec![0.0]),
        "a 50-unit disc 150 units past the wall never reaches the observer"
    );
}

#[tokio::test]
async fn recipient_reached_by_neither_token_nor_glow_gets_no_frame() {
    // The whole walk runs behind the wall and the torch's 10-unit glow never crosses it: no
    // position sample is visible and no disc reaches, so the frame is suppressed entirely —
    // the recipient learns nothing, not even that a light moved.
    let (room, _, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    let mut frame = torch_walk(scene_id, now, 10.0);
    if let ServerMsg::MoveStream {
        samples,
        mover_light,
        ..
    } = &mut frame
    {
        samples[0].pos = [150.0, 60.0];
        let lamps = mover_light.as_mut().unwrap();
        lamps[0] = lamp(0.0, [150.0, 60.0], 10.0);
    }
    assert!(clip_for(&frame, &obs_ctx, None, &room).await.is_none());
}

/// A glow-only frame: the whole walk runs behind the wall, but both torch discs reach across
/// it into the observer's sight. The recipient gets the frame with NO position samples and the
/// admitted light timeline, stopping where the last admitted light sample sits — a light that
/// sweeps past a doorway is seen as a light, never as the token carrying it.
#[tokio::test]
async fn glow_only_recipient_gets_a_frame_with_no_position_samples() {
    let (room, _, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    let mut frame = torch_walk(scene_id, now, 210.0);
    if let ServerMsg::MoveStream {
        samples,
        mover_light,
        ..
    } = &mut frame
    {
        samples[0].pos = [150.0, 60.0];
        let lamps = mover_light.as_mut().unwrap();
        lamps[0] = lamp(0.0, [150.0, 60.0], 210.0);
    }
    let out = clip_move_stream(
        &frame,
        &obs_ctx,
        None,
        &room,
        &crate::data::document::WorldCapDefaults::default(),
    )
    .await
    .expect("the glow reaches, so the frame is delivered");
    let ServerMsg::MoveStream {
        samples,
        mover_light,
        stop,
        duration_ms,
        ..
    } = out
    else {
        panic!("clip returns a MoveStream")
    };
    assert!(samples.is_empty(), "no position sample is in sight");
    let light = mover_light.expect("the admitted light timeline rides the frame");
    assert_eq!(
        light.iter().map(|l| l.t_ms).collect::<Vec<_>>(),
        vec![0.0, 1_000.0]
    );
    assert_eq!(
        stop,
        [250.0, 50.0],
        "stop is the last admitted light sample's position"
    );
    assert_eq!(duration_ms, 1_000.0);
}

/// Secrecy of an unlit token: with no wall, the stranger's dark walk at x∈[250,350] is in the
/// observer's line of sight but in cells no light reaches. A normal-vision observer gets NO
/// frame (the token is not visible, so nothing about its movement leaks), while a darkvision
/// observer within range and a plain GM receive the full walk.
#[tokio::test]
async fn an_unlit_token_in_line_of_sight_streams_only_to_darkvision_and_the_gm() {
    let dark_walk = |scene: Uuid, now: i64| {
        let mut frame = torch_walk(scene, now, 0.0);
        if let ServerMsg::MoveStream {
            samples,
            mover_light,
            ..
        } = &mut frame
        {
            samples[0].pos = [250.0, 50.0];
            samples[1].pos = [350.0, 50.0];
            *mover_light = None;
        }
        frame
    };

    let (room, gm_ctx, obs_ctx, scene_id, _) = setup_dark_clip_room(Some((50.0, 50.0)), None).await;
    let now = crate::ws::time::now_millis();
    assert!(
        clip_for(&dark_walk(scene_id, now), &obs_ctx, None, &room)
            .await
            .is_none(),
        "normal vision sees nothing in the dark, so the frame is suppressed"
    );
    assert_eq!(
        clip_for(&dark_walk(scene_id, now), &gm_ctx, None, &room).await,
        Some((2, None)),
        "a plain GM receives the full walk"
    );

    let (room, _, dv_ctx, scene_id, _) = setup_dark_clip_room_with_vision(
        (50.0, 50.0),
        None,
        json!([{ "mode": "darkvision", "range": 12.0 }]),
    )
    .await;
    assert_eq!(
        clip_for(&dark_walk(scene_id, now), &dv_ctx, None, &room).await,
        Some((2, None)),
        "darkvision within range sees the unlit walk"
    );
}

#[tokio::test]
async fn mover_and_plain_gm_receive_the_full_light_timeline() {
    let (room, gm_ctx, _, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    let frame = torch_walk(scene_id, now, 50.0);
    let mover_ctx = PermissionContext {
        user_id: Uuid::from_u128(0xAABB),
        world_role: crate::data::document::WorldRole::Player,
    };
    assert_eq!(
        clip_for(&frame, &mover_ctx, None, &room).await,
        Some((2, Some(vec![0.0, 1_000.0])))
    );
    assert_eq!(
        clip_for(&frame, &gm_ctx, None, &room).await,
        Some((2, Some(vec![0.0, 1_000.0]))),
        "a plain GM keeps full information, like `cost`"
    );
}

#[tokio::test]
async fn gm_see_as_admits_light_by_the_targets_vision() {
    let (room, gm_ctx, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    assert_eq!(
        clip_for(
            &torch_walk(scene_id, now, 50.0),
            &gm_ctx,
            Some(obs_ctx),
            &room
        )
        .await,
        Some((1, Some(vec![0.0]))),
        "see-as narrows the GM to exactly the target's admission"
    );
}

#[tokio::test]
async fn light_admission_reads_the_targets_own_in_flight_timeline() {
    // The observer's own walk (started before the torch's) carries its viewpoint past the wall
    // to (200,50) from t=200 after its start; the far lamp's one-unit disc at abs now+1100 is
    // inside that viewpoint's sight and is admitted through the timeline, exactly as its
    // position sample is (lit by the torch it sits on).
    let (room, _, obs_ctx, scene_id, _) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();
    register_timeline(
        &room,
        Uuid::from_u128(0xE002),
        obs_ctx.user_id,
        scene_id,
        now,
        vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [200.0, 50.0],
            },
        ],
    )
    .await;
    assert_eq!(
        clip_for(&torch_walk(scene_id, now + 100, 1.0), &obs_ctx, None, &room).await,
        Some((2, Some(vec![0.0, 1_000.0])))
    );
}

/// The own-move re-emit (`egress_loop`'s `MoveStream` arm) re-clips a concurrent stream's
/// light timeline through the same admission: A's lamp at abs `now` precedes R's walk and is
/// judged from R's committed (walled) viewpoint — dropped; at abs `now+1000` R's viewpoint is
/// past the wall — admitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn egress_reemit_re_admits_the_concurrent_streams_light_timeline() {
    let (room, _, obs_ctx, scene_id, repo) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let now = crate::ws::time::now_millis();

    let a_req = Uuid::from_u128(0xA11);
    let a_frame = ServerMsg::MoveStream {
        request_id: a_req,
        token_id: Uuid::from_u128(0xA),
        mover: Uuid::from_u128(0xAABB),
        scene: scene_id,
        start_server_ms: now as f64,
        duration_ms: 3_000.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0],
            },
            PosSample {
                t_ms: 1_000.0,
                pos: [250.0, 50.0],
            },
        ],
        mover_vision: None,
        mover_light: Some(vec![
            lamp(0.0, [150.0, 50.0], 10.0),
            lamp(1_000.0, [250.0, 50.0], 10.0),
        ]),
        cost: Some(2.0),
        truncated: Some(false),
    };
    room.register_stream_for_test(
        Uuid::from_u128(0xA),
        crate::ws::room::ActiveStream {
            mover: Uuid::from_u128(0xAABB),
            scene: scene_id,
            end_ms: now + 3_000,
            frame: Arc::new(a_frame),
        },
    )
    .await;

    let (rx, current_seq) = room.subscribe();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let (_etx, erx) = mpsc::channel::<Egress>(8);
    let egress = tokio::spawn(egress_loop(
        GatedSink {
            out: out_tx,
            credits: Arc::new(Semaphore::new(64)),
            acquiring: None,
        },
        rx,
        erx,
        EgressConnState {
            room: room.clone(),
            repo: repo.clone(),
            ctx: obs_ctx,
            current_seq,
            modules_dir: std::path::PathBuf::from("nonexistent-modules-dir"),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
        },
    ));
    let welcome = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(msg_text(&welcome)).unwrap()["type"],
        "welcome"
    );

    let r_start = now + 100;
    let r_frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0xB11),
        token_id: Uuid::from_u128(0xE002),
        mover: obs_ctx.user_id,
        scene: scene_id,
        start_server_ms: r_start as f64,
        duration_ms: 2_000.0,
        stop: [60.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 500.0,
                pos: [200.0, 50.0],
            },
            PosSample {
                t_ms: 2_000.0,
                pos: [250.0, 50.0],
            },
        ],
        // Present because the own-move re-emit keys on it; the clip reads viewpoints from the
        // position samples above, never these polygons.
        mover_vision: Some(vec![VisionSample {
            t_ms: 0.0,
            polygons: Vec::new(),
        }]),
        mover_light: None,
        cost: Some(0.1),
        truncated: Some(false),
    };
    let r_arc = Arc::new(r_frame);
    room.register_stream_for_test(
        Uuid::from_u128(0xE002),
        crate::ws::room::ActiveStream {
            mover: obs_ctx.user_id,
            scene: scene_id,
            end_ms: r_start + 2_000,
            frame: r_arc.clone(),
        },
    )
    .await;
    room.broadcast_aux_shared(r_arc);

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(msg_text(&first)).unwrap();
    assert_eq!(first["request_id"], json!(Uuid::from_u128(0xB11)));
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(msg_text(&second)).unwrap();
    assert_eq!(second["request_id"], json!(a_req), "A re-emitted");
    assert_eq!(second["samples"].as_array().unwrap().len(), 1);
    let light = second["mover_light"]
        .as_array()
        .expect("the re-emit carries A's admitted light timeline");
    assert_eq!(light.len(), 1);
    assert_eq!(light[0]["t_ms"], json!(1000.0));
    egress.abort();
}

/// The own-move re-emit yields a glow-only frame when the recipient's new viewpoint is
/// reached by a concurrent torch's glow but never by its bearer: A stands behind a second
/// wall at x=300, and only from R's viewpoint past the first wall does A's 120-unit glow
/// light a cell (250,50) in R's sight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn egress_reemit_yields_a_glow_only_frame_when_only_the_glow_reaches() {
    use crate::data::command::Operation;
    let (room, gm_ctx, obs_ctx, scene_id, repo) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    let mut far_wall = crate::data::document::tests::world_scoped_doc(
        room.world_id,
        Uuid::from_u128(0xE005),
        "wall",
    );
    far_wall.parent_id = Some(scene_id);
    far_wall.owner = Some(gm_ctx.user_id);
    far_wall.engine = Some(
        json!({ "seg": { "x1": 300, "y1": -500, "x2": 300, "y2": 500 }, "blocksSight": true }),
    );
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![Operation::Create { doc: far_wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let now = crate::ws::time::now_millis();

    let a_req = Uuid::from_u128(0xA11);
    let a_frame = ServerMsg::MoveStream {
        request_id: a_req,
        token_id: Uuid::from_u128(0xA),
        mover: Uuid::from_u128(0xAABB),
        scene: scene_id,
        start_server_ms: now as f64,
        duration_ms: 3_000.0,
        stop: [350.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [350.0, 50.0],
            },
            PosSample {
                t_ms: 1_000.0,
                pos: [350.0, 50.0],
            },
        ],
        mover_vision: None,
        mover_light: Some(vec![
            lamp(0.0, [350.0, 50.0], 120.0),
            lamp(1_000.0, [350.0, 50.0], 120.0),
        ]),
        cost: Some(0.0),
        truncated: Some(false),
    };
    room.register_stream_for_test(
        Uuid::from_u128(0xA),
        crate::ws::room::ActiveStream {
            mover: Uuid::from_u128(0xAABB),
            scene: scene_id,
            end_ms: now + 3_000,
            frame: Arc::new(a_frame),
        },
    )
    .await;

    let (rx, current_seq) = room.subscribe();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let (_etx, erx) = mpsc::channel::<Egress>(8);
    let egress = tokio::spawn(egress_loop(
        GatedSink {
            out: out_tx,
            credits: Arc::new(Semaphore::new(64)),
            acquiring: None,
        },
        rx,
        erx,
        EgressConnState {
            room: room.clone(),
            repo: repo.clone(),
            ctx: obs_ctx,
            current_seq,
            modules_dir: std::path::PathBuf::from("nonexistent-modules-dir"),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
        },
    ));
    let welcome = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(msg_text(&welcome)).unwrap()["type"],
        "welcome"
    );

    let r_start = now + 100;
    let r_frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0xB11),
        token_id: Uuid::from_u128(0xE002),
        mover: obs_ctx.user_id,
        scene: scene_id,
        start_server_ms: r_start as f64,
        duration_ms: 2_000.0,
        stop: [200.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 500.0,
                pos: [200.0, 50.0],
            },
            PosSample {
                t_ms: 2_000.0,
                pos: [200.0, 50.0],
            },
        ],
        mover_vision: Some(vec![VisionSample {
            t_ms: 0.0,
            polygons: Vec::new(),
        }]),
        mover_light: None,
        cost: Some(0.1),
        truncated: Some(false),
    };
    let r_arc = Arc::new(r_frame);
    room.register_stream_for_test(
        Uuid::from_u128(0xE002),
        crate::ws::room::ActiveStream {
            mover: obs_ctx.user_id,
            scene: scene_id,
            end_ms: r_start + 2_000,
            frame: r_arc.clone(),
        },
    )
    .await;
    room.broadcast_aux_shared(r_arc);

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(msg_text(&first)).unwrap();
    assert_eq!(first["request_id"], json!(Uuid::from_u128(0xB11)));
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(msg_text(&second)).unwrap();
    assert_eq!(
        second["request_id"],
        json!(a_req),
        "A re-emitted as a glow-only frame"
    );
    assert_eq!(
        second["samples"].as_array().unwrap().len(),
        0,
        "A's token stays behind the far wall"
    );
    let light = second["mover_light"]
        .as_array()
        .expect("the re-emit carries A's admitted light timeline");
    assert_eq!(light.len(), 1);
    assert_eq!(light[0]["t_ms"], json!(1000.0));
    assert_eq!(second["stop"], json!([350.0, 50.0]));
    egress.abort();
}
