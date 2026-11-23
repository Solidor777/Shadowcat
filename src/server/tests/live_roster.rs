//! The live roster: `Welcome::online_users` for a late joiner, and `ServerMsg::MemberProfile`
//! delivery when a member changes their account profile.
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

    let hash = hash_password("pw").unwrap();
    let stranger = h
        .repo
        .create_user("stranger", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    h.repo
        .create_world_owned("elsewhere", stranger, 0)
        .await
        .unwrap();
    let res = reqwest::Client::new()
        .post(format!("http://{}/api/login", h.addr))
        .json(&serde_json::json!({ "username": "stranger", "password": "pw" }))
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

    put_profile(
        &h,
        &cookie,
        serde_json::json!({ "colour": "#112233", "pronouns": null }),
    )
    .await;
    assert_no_member_profile(&mut gm, Duration::from_millis(500)).await;
}

/// A small valid PNG.
fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
