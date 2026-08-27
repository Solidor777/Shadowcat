use super::*;

#[test]
fn server_role_serde_round_trips_snake_case() {
    assert_eq!(
        serde_json::to_value(ServerRole::Admin).unwrap(),
        serde_json::json!("admin")
    );
    let r: ServerRole = serde_json::from_value(serde_json::json!("user")).unwrap();
    assert_eq!(r, ServerRole::User);
    assert_eq!(ServerRole::Admin.as_str(), "admin");
}
