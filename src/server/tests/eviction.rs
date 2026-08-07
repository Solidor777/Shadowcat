//! `ServerMsg::Evicted` delivery: per-user targeting and terminal close.
use shadowcat_test_support as common;

use common::drain_until_type;
use futures_util::StreamExt;
use shadowcat::ws::protocol::ServerMsg;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[tokio::test]
async fn world_delete_removes_asset_dir_and_room() {
    let h = common::spawn().await;
    let res = h
        .upload("a.png", "image/png", common::PNG_1X1.to_vec())
        .await;
    assert!(res.status().is_success(), "seed upload: {}", res.status());
    let mut ws = h.connect().await;
    drain_until_type(&mut ws, "welcome").await;

    let world_dir = h.assets_dir.join(h.world.to_string());
    assert!(
        std::fs::metadata(&world_dir).is_ok(),
        "world asset dir exists after upload"
    );

    // The seeded user is the world's GM; require_gm admits them.
    let res = h
        .client
        .delete(format!("http://{}/api/worlds/{}", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NO_CONTENT);

    // Rows first, files second: the world's asset directory is gone.
    assert!(std::fs::metadata(&world_dir).is_err());
    // Room dropped from the registry, and re-creation is refused because the
    // world ROW is gone while the tombstone has been lifted (finish_delete ran).
    assert!(h.ws.rooms.get(h.world).is_none());
    assert!(h
        .ws
        .rooms
        .get_or_create(h.repo.as_ref(), h.world)
        .await
        .unwrap()
        .is_none());

    // The live socket received the world-wide eviction, then closed.
    let evicted = drain_until_type(&mut ws, "evicted").await;
    assert!(evicted["user"].is_null());
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("socket should close promptly after the evicted frame");
    match next {
        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
        Some(Ok(other)) => panic!("expected close after evicted frame, got {other:?}"),
    }
}

#[tokio::test]
async fn world_delete_failure_lifts_tombstone() {
    let h = common::spawn().await;
    let hash = shadowcat::auth::password::hash_password("pw").unwrap();
    h.repo
        .create_user(
            "root",
            Some(&hash),
            shadowcat::auth::role::ServerRole::Admin,
            0,
        )
        .await
        .unwrap();
    let admin = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = admin
        .post(format!("http://{}/api/login", h.addr))
        .json(&serde_json::json!({ "username": "root", "password": "pw" }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let ghost = Uuid::new_v4();
    let res = admin
        .delete(format!("http://{}/api/worlds/{ghost}", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);

    // The failed delete tombstoned `ghost` on entry; the failure path must
    // lift it. Materialize a world under that SAME id — a leaked tombstone
    // would refuse room creation here.
    sqlx::query(
        "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, 'G', 0, 0, 0)",
    )
    .bind(ghost.to_string())
    .execute(h.repo.pool())
    .await
    .unwrap();
    assert!(h
        .ws
        .rooms
        .get_or_create(h.repo.as_ref(), ghost)
        .await
        .unwrap()
        .is_some());
    // Live worlds were never affected.
    assert!(h
        .ws
        .rooms
        .get_or_create(h.repo.as_ref(), h.world)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn user_delete_revokes_sessions_and_kicks() {
    let h = common::spawn().await;
    let (player_id, player_cookie) = h.add_player("victim").await;
    // The session is live before the deletion.
    let victim = reqwest::Client::new();
    let me = victim
        .get(format!("http://{}/api/me", h.addr))
        .header("cookie", &player_cookie)
        .send()
        .await
        .unwrap();
    assert!(me.status().is_success());
    let mut ws = h.connect_as(&player_cookie).await;
    drain_until_type(&mut ws, "welcome").await;

    // An admin performs the deletion.
    let hash = shadowcat::auth::password::hash_password("pw").unwrap();
    h.repo
        .create_user(
            "root",
            Some(&hash),
            shadowcat::auth::role::ServerRole::Admin,
            0,
        )
        .await
        .unwrap();
    let admin = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = admin
        .post(format!("http://{}/api/login", h.addr))
        .json(&serde_json::json!({ "username": "root", "password": "pw" }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let res = admin
        .delete(format!("http://{}/api/users/{player_id}", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NO_CONTENT);

    // The cookie died inside the delete transaction.
    let me = victim
        .get(format!("http://{}/api/me", h.addr))
        .header("cookie", &player_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The live socket received a TARGETED eviction, then closed.
    let evicted = drain_until_type(&mut ws, "evicted").await;
    assert_eq!(
        evicted["user"].as_str(),
        Some(player_id.to_string().as_str())
    );
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("socket should close promptly after the evicted frame");
    match next {
        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
        Some(Ok(other)) => panic!("expected close after evicted frame, got {other:?}"),
    }
}

#[tokio::test]
async fn evicted_frame_targets_and_closes() {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    drain_until_type(&mut ws, "welcome").await;

    let room = h.ws.rooms.get(h.world).expect("room exists after join");

    // Targeted at a DIFFERENT user: must not be delivered and must not close
    // this connection.
    room.broadcast_aux(ServerMsg::Evicted {
        user: Some(Uuid::new_v4()),
    });
    // Targeted at nobody (world deletion): delivered to everyone, then closed.
    room.broadcast_aux(ServerMsg::Evicted { user: None });

    // The next evicted frame this connection sees must be the world-wide one —
    // a delivered targeted-at-other frame would surface here with a non-null
    // user and fail the assert.
    let evicted = drain_until_type(&mut ws, "evicted").await;
    assert!(
        evicted["user"].is_null(),
        "targeted frame leaked to a non-target: {evicted}"
    );

    // The connection terminates right after: a Close frame or stream end.
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("socket should close promptly after the evicted frame");
    match next {
        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
        Some(Ok(other)) => panic!("expected close after evicted frame, got {other:?}"),
    }
}
