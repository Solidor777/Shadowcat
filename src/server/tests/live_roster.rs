//! The live roster: `Welcome::online_users` for a late joiner, and `ServerMsg::MemberProfile`
//! delivery when a member changes their account profile or is seated in the world.
use shadowcat_test_support as common;

use common::drain_until_type;
use futures_util::StreamExt;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Reads frames from `ws` for `window` and panics if any is a `member_profile` frame.
async fn assert_no_member_profile<S>(ws: &mut S, window: Duration)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if let Message::Text(text) = frame {
            let v: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
            assert_ne!(v["type"], "member_profile", "unexpected frame: {v}");
        }
    }
}

/// `PUT /api/account/profile` as the account behind `cookie`.
async fn put_profile(h: &common::Harness, cookie: &str, body: serde_json::Value) {
    let res = reqwest::Client::new()
        .put(format!("http://{}/api/account/profile", h.addr))
        .header("cookie", cookie)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
}

/// A late joiner's `Welcome` lists a member who connected before it, and a member who has
/// since disconnected is absent from the next joiner's `Welcome`.
#[tokio::test]
async fn welcome_lists_the_members_already_online() {
    let h = common::spawn().await;
    let (player, cookie) = h.add_player("late-player").await;

    let mut gm = h.connect().await;
    let gm_welcome = drain_until_type(&mut gm, "welcome").await;
    // The receiving connection counts itself.
    assert_eq!(
        gm_welcome["online_users"],
        serde_json::json!([h.user.to_string()])
    );

    let mut late = h.connect_as(&cookie).await;
    let welcome = drain_until_type(&mut late, "welcome").await;
    let online: Vec<String> = welcome["online_users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        online.contains(&h.user.to_string()),
        "GM missing: {online:?}"
    );
    assert!(
        online.contains(&player.to_string()),
        "self missing: {online:?}"
    );
    assert_eq!(online.len(), 2);

    late.close(None).await.unwrap();
    drop(late);
    drain_until_type(&mut gm, "presence").await; // the late player's offline transition
    let mut rejoin = h.connect().await;
    let welcome = drain_until_type(&mut rejoin, "welcome").await;
    assert_eq!(
        welcome["online_users"],
        serde_json::json!([h.user.to_string()])
    );
}

/// A profile change reaches another connected member of the same world without a reconnect,
/// for the profile route and for the avatar routes.
#[tokio::test]
async fn profile_change_reaches_another_connected_member() {
    let h = common::spawn().await;
    let (player, cookie) = h.add_player("profile-player").await;
    let mut gm = h.connect().await;
    drain_until_type(&mut gm, "welcome").await;

    put_profile(
        &h,
        &cookie,
        serde_json::json!({ "colour": "#336699", "pronouns": "they/them" }),
    )
    .await;
    let frame = drain_until_type(&mut gm, "member_profile").await;
    assert_eq!(frame["user"].as_str(), Some(player.to_string().as_str()));
    assert_eq!(frame["colour"], "#336699");
    assert_eq!(frame["pronouns"], "they/them");
    assert_eq!(frame["has_avatar"], false);

    let res = reqwest::Client::new()
        .put(format!("http://{}/api/account/avatar", h.addr))
        .header("cookie", &cookie)
        .body(tiny_png())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let frame = drain_until_type(&mut gm, "member_profile").await;
    assert_eq!(frame["has_avatar"], true);
    assert_eq!(frame["colour"], "#336699");

    let res = reqwest::Client::new()
        .delete(format!("http://{}/api/account/avatar", h.addr))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let frame = drain_until_type(&mut gm, "member_profile").await;
    assert_eq!(frame["has_avatar"], false);
}

/// A user seated only in another world never has their profile change delivered to this
/// world's connections: the broadcast follows seating, the roster's own boundary.
#[tokio::test]
async fn profile_change_is_not_sent_to_a_world_the_user_is_not_seated_in() {
    let h = common::spawn().await;
    let mut gm = h.connect().await;
    drain_until_type(&mut gm, "welcome").await;

    let (stranger, cookie) = new_account(&h, "stranger").await;
    h.repo
        .create_world_owned("elsewhere", stranger, 0)
        .await
        .unwrap();

    put_profile(
        &h,
        &cookie,
        serde_json::json!({ "colour": "#112233", "pronouns": null }),
    )
    .await;
    assert_no_member_profile(&mut gm, Duration::from_millis(500)).await;
}

/// Creates an account seated in no world and logs it in; returns `(user_id, session-cookie)`.
async fn new_account(h: &common::Harness, username: &str) -> (uuid::Uuid, String) {
    let hash = hash_password("pw").unwrap();
    let id = h
        .repo
        .create_user(username, Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let res = reqwest::Client::new()
        .post(format!("http://{}/api/login", h.addr))
        .json(&serde_json::json!({ "username": username, "password": "pw" }))
        .send()
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    (id, cookie)
}

/// Asserts `frame` is the roster row of `user` named `username`.
fn assert_roster_row(frame: &serde_json::Value, user: uuid::Uuid, username: &str) {
    assert_eq!(frame["user"].as_str(), Some(user.to_string().as_str()));
    assert_eq!(frame["username"], username);
    assert_eq!(frame["colour"], "#336699");
    assert_eq!(frame["pronouns"], "she/her");
    assert_eq!(frame["has_avatar"], false);
}

/// A GM seating an existing account through the members route reaches a connected GM's roster
/// without a reconnect, carrying the username and stored profile.
#[tokio::test]
async fn gm_seating_a_member_announces_them_to_a_connected_client() {
    let h = common::spawn().await;
    let (newcomer, cookie) = new_account(&h, "seated-by-gm").await;
    put_profile(
        &h,
        &cookie,
        serde_json::json!({ "colour": "#336699", "pronouns": "she/her" }),
    )
    .await;
    let mut gm = h.connect().await;
    drain_until_type(&mut gm, "welcome").await;

    let res = reqwest::Client::new()
        .post(format!("http://{}/api/worlds/{}/members", h.addr, h.world))
        .header("cookie", &h.cookie)
        .json(&serde_json::json!({ "user": newcomer, "role": "player" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    let frame = drain_until_type(&mut gm, "member_profile").await;
    assert_roster_row(&frame, newcomer, "seated-by-gm");
}

/// Redeeming an invite reaches a connected GM's roster without a reconnect.
#[tokio::test]
async fn accepting_an_invite_announces_the_new_member_to_a_connected_client() {
    let h = common::spawn().await;
    let (newcomer, cookie) = new_account(&h, "seated-by-invite").await;
    put_profile(
        &h,
        &cookie,
        serde_json::json!({ "colour": "#336699", "pronouns": "she/her" }),
    )
    .await;
    let mut gm = h.connect().await;
    drain_until_type(&mut gm, "welcome").await;

    let client = reqwest::Client::new();
    let minted: serde_json::Value = client
        .post(format!("http://{}/api/worlds/{}/invites", h.addr, h.world))
        .header("cookie", &h.cookie)
        .json(&serde_json::json!({ "role": "player" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let res = client
        .post(format!("http://{}/api/invites/accept", h.addr))
        .header("cookie", &cookie)
        .json(&serde_json::json!({ "code": minted["code"] }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "accept failed: {}", res.status());

    let frame = drain_until_type(&mut gm, "member_profile").await;
    assert_roster_row(&frame, newcomer, "seated-by-invite");
}

/// A client connected to a different world gets no roster row when someone is seated here.
#[tokio::test]
async fn seating_is_not_announced_to_another_world() {
    let h = common::spawn().await;
    let (newcomer, _cookie) = new_account(&h, "seated-elsewhere-watcher").await;
    let (other_gm, other_cookie) = new_account(&h, "other-gm").await;
    let other_world = h
        .repo
        .create_world_owned("elsewhere", other_gm, 0)
        .await
        .unwrap();
    let url = format!("ws://{}/ws?world={}", h.addr, other_world.id);
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url)
            .unwrap();
    req.headers_mut()
        .insert("cookie", other_cookie.parse().unwrap());
    let (mut other, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    drain_until_type(&mut other, "welcome").await;

    let res = reqwest::Client::new()
        .post(format!("http://{}/api/worlds/{}/members", h.addr, h.world))
        .header("cookie", &h.cookie)
        .json(&serde_json::json!({ "user": newcomer, "role": "player" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert_no_member_profile(&mut other, Duration::from_millis(500)).await;
}

/// A small valid PNG.
fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
