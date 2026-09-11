use super::*;
use crate::data::document::RoleCaps;

#[test]
fn project_role_caps_drops_other_roles_entries() {
    let mut caps = RoleCaps::default();
    caps.all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".to_string());
    caps.all
        .entry(WorldRole::Gm)
        .or_default()
        .insert("core:manage_world".to_string());
    caps.by_type
        .entry("note".to_string())
        .or_default()
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".to_string());
    caps.by_type
        .entry("scene".to_string())
        .or_default()
        .entry(WorldRole::Gm)
        .or_default()
        .insert("core:create".to_string());

    let mine = project_role_caps_for(&caps, WorldRole::Player);
    assert_eq!(mine.all, ["core:create".to_string()].into_iter().collect());
    assert_eq!(mine.by_type.len(), 1);
    assert!(mine.by_type.contains_key("note"));
    assert!(!mine.by_type.contains_key("scene"));
}
