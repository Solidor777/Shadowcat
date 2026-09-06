use super::*;

#[test]
fn engine_whole_and_subpaths_require_write_fields() {
    assert_eq!(required_cap_for_path("/engine"), Some(cap::WRITE_FIELDS));
    assert_eq!(required_cap_for_path("/engine/x"), Some(cap::WRITE_FIELDS));
    assert_eq!(
        required_cap_for_path("/engine/vision/0/range"),
        Some(cap::WRITE_FIELDS)
    );
}

#[test]
fn engine_boundary_neighbor_does_not_match() {
    // `/engine_x` must not fall under the `/engine` prefix rule.
    assert_eq!(required_cap_for_path("/engine_x"), None);
}

#[test]
fn name_requires_write_fields_but_is_a_leaf() {
    assert_eq!(required_cap_for_path("/name"), Some(cap::WRITE_FIELDS));
    // `/name` has no sub-paths — a leaf value, not a container.
    assert_eq!(required_cap_for_path("/name/first"), None);
    assert_eq!(required_cap_for_path("/named"), None);
}

#[test]
fn base_is_server_owned_and_maps_to_no_capability() {
    // `/base` is server-owned: derived at Create, refreshed by merge writes — so,
    // like `/source`, no capability reaches it.
    assert_eq!(required_cap_for_path("/base"), None);
    assert_eq!(required_cap_for_path("/base/system/hp"), None);
    assert_eq!(required_cap_for_path("/base/embedded/actor/0/name"), None);
}

#[test]
fn base_boundary_neighbor_does_not_match() {
    assert_eq!(required_cap_for_path("/based"), None);
}

#[test]
fn owner_requires_edit_permissions_and_is_a_leaf() {
    // Re-assigning ownership is an access-control write: EDIT_PERMISSIONS,
    // which the `DocRole::Owner` floor does not include.
    assert_eq!(required_cap_for_path("/owner"), Some(cap::EDIT_PERMISSIONS));
    assert_eq!(required_cap_for_path("/owner/id"), None);
    assert_eq!(required_cap_for_path("/owners"), None);
}

#[test]
fn the_writable_band_set_is_the_redactable_set_minus_server_owned_base() {
    // The two functions are not per-string equal and must not be tested that way
    // (`/system/` is `WRITE_FIELDS` for one and unclassifiable for the other). What
    // must hold is their BAND SETS: every client-writable band is redactable, and the
    // redactable set carries exactly one extra band — `base`, redactable at egress but
    // server-owned, so writable by no client capability.
    //
    // The universe is HARDCODED, never derived from `REDACTABLE_BANDS`: probing only
    // the constant's own contents would make the assertion definitionally true for any
    // contents, and would not notice a band silently added to one side.
    let universe = [
        "name",
        "engine",
        "system",
        "base",
        "id",
        "scope",
        "doc_type",
        "schema_version",
        "source",
        "owner",
        "permissions",
        "parent_id",
        "embedded",
        "created_at",
        "updated_at",
    ];
    let writable: Vec<&str> = universe
        .into_iter()
        .filter(|f| required_cap_for_path(&format!("/{f}")) == Some(cap::WRITE_FIELDS))
        .collect();
    let redactable: Vec<&str> = universe
        .into_iter()
        .filter(|f| redaction_target(&format!("/{f}")).is_some())
        .collect();
    assert_eq!(
        writable,
        ["name", "engine", "system"],
        "the client-writable content band set changed"
    );
    assert_eq!(
        redactable,
        ["name", "engine", "system", "base"],
        "the redactable content band set changed"
    );
}

#[test]
fn source_is_immutable_no_cap() {
    // `/source` maps to no capability, so an Update targeting it is Forbidden for everyone.
    assert_eq!(required_cap_for_path("/source"), None);
    assert_eq!(required_cap_for_path("/source/id"), None);
}

#[test]
fn targets_engine_band_admits_the_root_and_embedded_engine_bands_only() {
    assert!(targets_engine_band("/engine"));
    assert!(targets_engine_band("/engine/combat"));
    assert!(targets_engine_band("/embedded/actor/0/engine"));
    assert!(targets_engine_band("/embedded/actor/0/engine/faction"));
    assert!(targets_engine_band(
        "/embedded/actor/0/embedded/item/2/engine/x"
    ));
    // Other bands, the envelope, and boundary neighbours.
    assert!(!targets_engine_band("/system"));
    assert!(!targets_engine_band("/system/engine"));
    assert!(!targets_engine_band("/name"));
    assert!(!targets_engine_band("/engine_x"));
    assert!(!targets_engine_band("/engine/"));
    assert!(!targets_engine_band("/permissions/default"));
    assert!(!targets_engine_band("/base/engine"));
    // A whole collection or a whole record is not an engine-band write.
    assert!(!targets_engine_band("/embedded/actor"));
    assert!(!targets_engine_band("/embedded/actor/0"));
    assert!(!targets_engine_band("/embedded/actor/0/system/hp"));
}

#[test]
fn targets_engine_band_and_writes_a_content_band_share_the_band_prefix_rule() {
    // Every engine-band path is a content-band write; the converse fails only
    // on the other two bands.
    for p in ["/engine", "/engine/x", "/engine/a/b"] {
        assert!(targets_engine_band(p) && writes_a_content_band(p), "{p}");
    }
    for p in ["/system", "/system/x", "/name"] {
        assert!(!targets_engine_band(p) && writes_a_content_band(p), "{p}");
    }
}
