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
fn base_whole_and_subpaths_require_write_fields() {
    assert_eq!(required_cap_for_path("/base"), Some(cap::WRITE_FIELDS));
    assert_eq!(
        required_cap_for_path("/base/system/hp"),
        Some(cap::WRITE_FIELDS)
    );
    assert_eq!(
        required_cap_for_path("/base/embedded/actor/0/name"),
        Some(cap::WRITE_FIELDS)
    );
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
fn the_write_fields_band_set_equals_the_redactable_band_set() {
    // The two functions are not per-string equal and must not be tested that way
    // (`/system/` is `WRITE_FIELDS` for one and unclassifiable for the other). What
    // must hold is that they admit the same BAND SET, so a fifth band cannot become
    // redactable without also becoming writable under the same capability.
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
        writable, redactable,
        "the WRITE_FIELDS set and the redactable set diverged"
    );
    assert_eq!(
        writable,
        ["name", "engine", "system", "base"],
        "both sets changed together but are no longer the four content bands"
    );
}

#[test]
fn source_is_immutable_no_cap() {
    // `/source` maps to no capability, so an Update targeting it is Forbidden for everyone.
    assert_eq!(required_cap_for_path("/source"), None);
    assert_eq!(required_cap_for_path("/source/id"), None);
}
