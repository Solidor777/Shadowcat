# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

- **`property_overrides` keys are not restricted to the four egress-special-cased fields; a
  self-targeting `/permissions` key silently substitutes the fail-closed default permissions
  object for a redacted viewer.** `validate_property_overrides`
  (`src/server/src/data/validation.rs:332-337`, gated on both Create and Update ingress —
  `src/server/src/data/sqlite.rs:2041,2409`) checks only that a key is a well-formed non-empty JSON
  pointer (starts with `/`, no trailing `/`) — nothing restricts which top-level `Document` field it
  names. `filter_properties` (`src/server/src/data/permission.rs:701-756`) special-cases only
  `/system`, `/engine`, `/name`, `/base` (nulled in place, lines 746-751); any other hidden
  `property_overrides` pointer — including one naming `/permissions` or `/permissions/...` itself —
  falls through to the generic `strip_pointer` (`src/server/src/data/permission.rs:959-978`), which
  does a plain `Map::remove` on whatever top-level key the pointer names. For a single-token
  `/permissions` pointer, this removes the entire `permissions` object from the serialized document
  before `serde_json::from_value` re-deserializes it. Because `Document.permissions` carries
  `#[serde(default)]` (`src/server/src/data/document.rs:479-480`) and `PermissionSet` derives
  `Default` with `default: DocRole::None` (`src/server/src/data/document.rs:415,418-420` —
  fail-closed), re-deserialization does not panic; it silently substitutes the fail-closed default
  `PermissionSet` for the real one.
  - **A NESTED `/permissions/...` key is worse: it PANICS the request.** `PermissionSet`'s `default`,
    `users` and `property_overrides` fields carry **no** `#[serde(default)]` — only `capabilities`
    and `gm_role` do (`src/server/src/data/document.rs:417-439`). So an override naming
    `/permissions/default`, `/permissions/users` or `/permissions/property_overrides` strips a
    REQUIRED field while the enclosing `permissions` object survives, leaving a value that cannot
    deserialize as `PermissionSet` — and the tail of `filter_properties` is
    `serde_json::from_value(whole).expect("filtered document deserializes")`
    (`src/server/src/data/permission.rs:755`). The `expect` is not a cold-path assertion:
    `filter_properties` runs per-recipient on the WS broadcast egress path (`filter_command`,
    `src/server/src/data/permission.rs:833-851`), on FTS search hits
    (`src/server/src/data/sqlite.rs:2785`), and on the HTTP get-document routes
    (`src/server/src/http/routes.rs:975,1026`). Any recipient who cannot see the offending tier
    crashes the request handling their read — i.e. a denial-of-service against every such reader of
    that document, authorable by one holder of `cap::EDIT_PERMISSIONS`.
  - **Reachability:** requires `cap::EDIT_PERMISSIONS` on the document's `doc_type` — every GM has
    this; a non-GM could hold it only via an explicit `by_role`/`users` capability grant. No UI path
    in this codebase constructs a `property_overrides` key outside `/system`, `/engine`, `/name`,
    `/base` today; a raw protocol Update/Create message is not otherwise blocked from doing so.
  - **Effect:** a viewer who cannot see the offending override tier receives a document whose
    `permissions` field is the fail-closed default rather than the real one — a data-integrity
    defect (e.g. a client computing `isHidden`-style checks from the received `permissions` would
    misreport). **Not an authorization bypass**: write authorization always re-resolves server-side
    against the stored row, never against a redacted client-facing copy, and the substituted default
    is strictly more restrictive than the real value, never less.
  - No fix shape decided — restricting legal override targets to a whitelist, null-in-place (like
    the four special-cased fields) instead of stripping, and rejecting only self-referential keys
    each have different consequences for any document already carrying an unusual override path.
    Note that an ingress-only fix does not reach documents already stored, and that the `expect`
    at `permission.rs:755` is a second, independent hardening target: any future gap between what
    ingress admits and what egress can re-deserialize lands on that same panic.
