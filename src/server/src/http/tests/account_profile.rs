//! Tests for the self-service account profile and avatar routes and the roster fields they feed.

use super::*;
use axum::body::Bytes;

/// A small valid PNG.
fn tiny_png() -> Vec<u8> {
    png_of(8, 8)
}

/// A PNG of `w` x `h` solid pixels.
fn png_of(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A world with a GM (`gm`), a player (`pl`) and a non-member (`out`), each logged in.
struct Fixture {
    state: AppState,
    world: String,
    gm: axum_test::TestServer,
    pl: axum_test::TestServer,
    out: axum_test::TestServer,
    gm_id: Uuid,
    pl_id: Uuid,
    out_id: Uuid,
}

/// Seeds the three accounts, a world owned by the GM, and seats the player.
async fn fixture() -> Fixture {
    let state = initialized_state().await;
    let gm_id = seed_user(&state, "gm").await;
    let pl_id = seed_user(&state, "pl").await;
    let out_id = seed_user(&state, "out").await;
    let gm = login_server(&state, "gm").await;
    let pl = login_server(&state, "pl").await;
    let out = login_server(&state, "out").await;
    let world: serde_json::Value = gm
        .post("/api/worlds")
        .json(&serde_json::json!({ "name": "W" }))
        .await
        .json();
    let world = world["id"].as_str().unwrap().to_string();
    gm.post(&format!("/api/worlds/{world}/members"))
        .json(&serde_json::json!({ "user": pl_id, "role": "player" }))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    Fixture {
        state,
        world,
        gm,
        pl,
        out,
        gm_id,
        pl_id,
        out_id,
    }
}

/// The roster entry for `user` as the GM sees it.
async fn roster_entry(f: &Fixture, user: Uuid) -> serde_json::Value {
    let members: serde_json::Value =
        f.gm.get(&format!("/api/worlds/{}/members", f.world))
            .await
            .json();
    members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["user"] == user.to_string())
        .unwrap()
        .clone()
}

#[tokio::test]
async fn profile_round_trips_into_the_roster_for_every_member() {
    let f = fixture().await;
    f.pl.put("/api/account/profile")
        .json(&serde_json::json!({ "colour": "#336699", "pronouns": "  they/them " }))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let entry = roster_entry(&f, f.pl_id).await;
    assert_eq!(entry["colour"], "#336699");
    assert_eq!(entry["pronouns"], "they/them");
    assert_eq!(entry["has_avatar"], false);

    // `null` clears a field.
    f.pl.put("/api/account/profile")
        .json(&serde_json::json!({ "colour": null, "pronouns": null }))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let entry = roster_entry(&f, f.pl_id).await;
    assert!(entry["colour"].is_null() && entry["pronouns"].is_null());
}

/// The colour check is the shared `validate_hex_color`; a CSS named colour must not slip past a
/// looser check.
#[tokio::test]
async fn profile_rejects_a_non_hex_colour_and_a_bad_pronouns_value_with_422() {
    let f = fixture().await;
    for body in [
        serde_json::json!({ "colour": "red", "pronouns": null }),
        serde_json::json!({ "colour": "#12345", "pronouns": null }),
        serde_json::json!({ "colour": null, "pronouns": "x".repeat(33) }),
        serde_json::json!({ "colour": null, "pronouns": "a\nb" }),
    ] {
        f.pl.put("/api/account/profile")
            .json(&body)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let record = f.state.repo.user_by_id(f.pl_id).await.unwrap().unwrap();
    assert!(record.colour.is_none() && record.pronouns.is_none());
}

#[tokio::test]
async fn profile_and_avatar_routes_require_authentication() {
    let state = initialized_state().await;
    let anon = anon_server(&state).await;
    anon.put("/api/account/profile")
        .json(&serde_json::json!({ "colour": null, "pronouns": null }))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    anon.put("/api/account/avatar")
        .bytes(Bytes::from(tiny_png()))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    anon.delete("/api/account/avatar")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn avatar_upload_is_served_to_other_members_flags_the_roster_and_clears() {
    let f = fixture().await;
    let png = tiny_png();
    f.pl.put("/api/account/avatar")
        .bytes(Bytes::from(png.clone()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(roster_entry(&f, f.pl_id).await["has_avatar"], true);

    let url = format!("/api/worlds/{}/members/{}/avatar", f.world, f.pl_id);
    let res = f.gm.get(&url).await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "image/png");
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(res.as_bytes().to_vec(), png);

    f.pl.delete("/api/account/avatar")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(roster_entry(&f, f.pl_id).await["has_avatar"], false);
    f.gm.get(&url).await.assert_status(StatusCode::NOT_FOUND);
}

/// The stored type comes from the bytes, never the request header.
#[tokio::test]
async fn avatar_content_type_is_sniffed_not_trusted() {
    let f = fixture().await;
    f.pl.put("/api/account/avatar")
        .content_type("image/jpeg")
        .bytes(Bytes::from(tiny_png()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let url = format!("/api/worlds/{}/members/{}/avatar", f.world, f.pl_id);
    assert_eq!(f.gm.get(&url).await.header("content-type"), "image/png");
}

#[tokio::test]
async fn avatar_over_128_kib_is_refused_with_413_and_stores_nothing() {
    let f = fixture().await;
    // Not decodable on purpose: only the size cap can be what refuses it.
    let big = vec![0u8; routes::MAX_AVATAR_BYTES + 1];
    f.pl.put("/api/account/avatar")
        .bytes(Bytes::from(big))
        .await
        .assert_status(StatusCode::PAYLOAD_TOO_LARGE);
    assert!(f.state.repo.avatar_for(f.pl_id).await.unwrap().is_none());
}

#[tokio::test]
async fn avatar_that_is_not_png_jpeg_or_webp_is_refused_with_422() {
    let f = fixture().await;
    // A declared image type does not make non-image bytes acceptable.
    f.pl.put("/api/account/avatar")
        .content_type("image/png")
        .bytes(Bytes::from_static(b"definitely not an image"))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    // An image the sniffer knows but the avatar allow-list excludes.
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&[0u8; 32]);
    f.pl.put("/api/account/avatar")
        .bytes(Bytes::from(gif))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(f.state.repo.avatar_for(f.pl_id).await.unwrap().is_none());
}

/// A PNG under the byte cap whose pixel buffer exceeds the decode bound must be refused at
/// upload, not stored and later decoded by every viewer's browser.
#[tokio::test]
async fn avatar_decompression_bomb_is_refused_with_422() {
    let f = fixture().await;
    let bomb = png_of(1, crate::data::asset::process::MAX_DECODE_AXIS_PX + 1);
    assert!(
        bomb.len() <= routes::MAX_AVATAR_BYTES,
        "fixture must fit the cap"
    );
    f.pl.put("/api/account/avatar")
        .bytes(Bytes::from(bomb))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(f.state.repo.avatar_for(f.pl_id).await.unwrap().is_none());
}

/// The avatar route mirrors the roster boundary: a target who shares no world with the caller
/// is 404, the same answer as "no avatar"; a non-member caller is 403.
#[tokio::test]
async fn avatar_get_is_404_for_a_non_member_target_and_403_for_a_non_member_caller() {
    let f = fixture().await;
    // The outsider has an avatar, but is not a member of the GM's world.
    f.out
        .put("/api/account/avatar")
        .bytes(Bytes::from(tiny_png()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    f.gm.get(&format!(
        "/api/worlds/{}/members/{}/avatar",
        f.world, f.out_id
    ))
    .await
    .assert_status(StatusCode::NOT_FOUND);

    // A member target with an avatar is visible to members, hidden from non-members.
    f.pl.put("/api/account/avatar")
        .bytes(Bytes::from(tiny_png()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    f.out
        .get(&format!(
            "/api/worlds/{}/members/{}/avatar",
            f.world, f.pl_id
        ))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    f.pl.get(&format!(
        "/api/worlds/{}/members/{}/avatar",
        f.world, f.gm_id
    ))
    .await
    .assert_status(StatusCode::NOT_FOUND);
}
