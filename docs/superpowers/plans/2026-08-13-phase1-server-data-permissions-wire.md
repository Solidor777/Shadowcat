# Phase 1 — Server: Data, Permissions, Wire — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make document redaction operate on content bands rather than arbitrary pointers, so no
`property_overrides` key can substitute a fail-closed permission set or panic the per-recipient
egress path, and tighten three client wire declarations against their Rust sources.

**Architecture:** One shared classifier in `data::permission` decides what a `property_overrides`
pointer means. Ingress calls it to reject anything it cannot classify; egress calls it to decide
null-in-place versus pointer-strip. Today those two paths duplicate that judgement, and the panic
this phase removes is what the duplication looks like when it drifts — so they must read **one
symbol**, not agree by inspection. `filter_properties` then returns a `Result` and every caller
fails **closed**: withhold, never guess, never panic.

**Tech Stack:** Rust (server crate `shadowcat`, `serde_json`, `sqlx`), TypeScript + Zod v3
(`@shadowcat/core`).

**Spec:** `docs/superpowers/specs/2026-08-13-debt-burndown-campaign-design.md`

**Ledger ids covered:** OB2, TD26, TD27, TD31, PW19.

## Global Constraints

- **The campaign directive in the spec's §1 is copied verbatim into every subagent's first prompt.**
- **Report channel, stated in every dispatch:** return the report as the Agent tool's result, or
  send it via SendMessage, or write it to a named document. Dispatches are launched **without** a
  `name` — naming an agent backgrounds it and its final text reaches nobody.
- **Per-item disposition.** Every task reports one line per ledger id it touches. "Category
  complete" is not an accepted report shape.
- **No suppressions.** `#[allow(...)]` and `#[expect(...)]` are both forbidden. Fix the code, or
  stop and ask.
- **Comments cite symbols, never file names or line numbers**, and never name a milestone, task id,
  sweep, date, repo-document pointer, or the code's own history.
- **No migrations.** SQL schema changes edit the single baseline in place. This phase needs none.
- **Cross-platform:** `std::path` only; no hardcoded separators.
- **Verification:** server changes run `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings`
  from `src/server/`. Client changes run `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` from the
  repo root. A shared wire-schema change runs the **full repo** test gate, never a filtered run —
  a typecheck alone misses the Zod runtime drop.
- **Branch:** `phase1-server-data-permissions-wire`, merged `--no-ff` to local `main`. No push
  until the sub-project completes. History is never rewritten.

## Model/Effort directives

- **Plan authored mainline** at Opus / effort high, by user decision, rather than dispatched to
  `sdd-plan-writer-opus` — the plan needed the code verifications and fork reasoning held in the
  authoring session that did not all survive into the spec text.
- **Execution loop:** Opus 5 at effort high, by user decision — this phase rewrites a
  per-recipient secrecy boundary and carries two buddy-check debates to broker to convergence.
- **Implementation:** `shadowcat-coder`, `effort: medium`. On BLOCKED, re-dispatch to
  `shadowcat-coder-opus` (`effort: high`) before escalating to the human.
- **Review:** `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`, `effort: high`, as a pair at
  every task gate. On shallow or uncertain findings, re-dispatch the `-opus` twin.
- Reviewers have **no shell** by directive: pre-generate the task diff to a file and relay gate
  output to them.

## Buddy-check directives

Pre-authorized by the user:

- **This plan document** is buddy-checked before Task 1 begins (PHASE = spec). Two blind reviewers,
  then a brokered debate to convergence. Findings fold into the plan before any code is written.
- **Directed question for that buddy-check: PW19.** The user has deferred the replay-redaction
  ruling to the reviewers rather than taking it on one analysis. Both reviewers argue independently
  whether resolving an `Update`'s visibility against the CURRENT permission set can leak in either
  direction; their convergence — or their stalemate, stated as such — goes to the user, who rules
  before Task 6 runs. Neither reviewer is told the other's position, and neither is told the
  authoring session's conclusion, which was that no leak exists either way.
- **Second directed question: Task 3's spec deviation.** The plan refines the spec's "error" policy
  for `list_documents` and `search` into "omit the item and log". Reviewers accept or reject it
  explicitly rather than passing over it.
- **Tasks 1 and 3** are buddy-checked (PHASE = code) — the classifier and the fail-closed egress
  conversion are the security boundary.
- Tasks 2, 4, 5, 6 take the standard two-reviewer gate.
- A **whole-branch** review runs before merge regardless.

---

## File Structure

| File | Responsibility in this phase |
|---|---|
| `src/server/src/data/permission.rs` | Owns the new band classifier, `RedactionError`, the `Result`-returning `filter_properties`, and `filter_command`'s fail-closed arms. Already the home of every egress redaction decision. |
| `src/server/src/data/validation.rs` | `validate_property_overrides` gains the classifier call. Ingress-only; no other change. |
| `src/server/src/http/routes.rs` | Two `filter_properties` call sites adopt their fail-closed policy. |
| `src/server/src/data/search.rs` | The index-build call site adopts its fail-closed policy. |
| `src/server/src/data/sqlite.rs` | The search-hit call site adopts its fail-closed policy. The two `validate_property_overrides` call sites need **no edit** — they already call it. |
| `src/client/core/src/wire.ts` | `WireFieldChange`/`FieldChangeSchema` require their value keys; `WireCapabilityGrants.by_role` narrows to a role-keyed map; `WireSearchHit.snippet` gains its exposure note. |
| `src/client/core/src/wire.test.ts` | Tests for both wire tightenings. |

**Band inventory, verified against the `Document` struct** — four content bands (`name`, `engine`,
`system`, `base`) and eleven structural fields (`id`, `scope`, `doc_type`, `schema_version`,
`source`, `owner`, `permissions`, `parent_id`, `embedded`, `created_at`, `updated_at`). The four
bands are exactly the set `filter_properties` already special-cases and exactly the set
`required_cap_for_path` maps to `cap::WRITE_FIELDS`. That coincidence is the design, not luck.

---

### Task 1: The shared redaction classifier

**Files:**
- Modify: `src/server/src/data/permission.rs` (add the classifier next to `required_cap_for_path`)
- Test: `src/server/src/data/permission.rs` (`mod tests` at the bottom of the same file)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub const REDACTABLE_BANDS: [&str; 4]`
  - `pub enum RedactionTarget { Band, Within }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub fn redaction_target(pointer: &str) -> Option<RedactionTarget>`

  Task 2 calls `redaction_target` for ingress rejection; Task 3 calls it for the egress branch.

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `src/server/src/data/permission.rs`:

```rust
#[test]
fn redaction_target_classifies_each_whole_band() {
    // The expectation is a HARDCODED list, never `REDACTABLE_BANDS` itself. Deriving the
    // expected value from the constant under test makes the assertion definitionally true
    // for any array contents — it would stay green if a band were renamed, which is the
    // exact "both paths wrong the same way" shape this suite exists to refuse.
    for band in ["name", "engine", "system", "base"] {
        let pointer = format!("/{band}");
        assert_eq!(
            redaction_target(&pointer),
            Some(RedactionTarget::Band),
            "{pointer} must classify as a whole band"
        );
    }
    // Pins the constant's contents independently, so a band added or renamed fails HERE
    // with a message naming the obligation, rather than silently widening what egress
    // is willing to remove.
    assert_eq!(
        REDACTABLE_BANDS,
        ["name", "engine", "system", "base"],
        "the band list changed: re-audit every redaction call site and this suite"
    );
}

#[test]
fn redaction_target_classifies_within_a_band() {
    for pointer in [
        "/system/hp",
        "/system/a/b/c",
        "/engine/vision",
        "/base/system/hp",
    ] {
        assert_eq!(
            redaction_target(pointer),
            Some(RedactionTarget::Within),
            "{pointer} must classify as within a band"
        );
    }
}

#[test]
fn redaction_target_refuses_every_structural_envelope_field() {
    // The eleven non-content fields of `Document`. Nothing may redact these: a
    // whole-key strip either substitutes a defaulted value or leaves a shape that
    // cannot deserialize.
    for field in [
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
    ] {
        assert_eq!(redaction_target(&format!("/{field}")), None, "/{field}");
        assert_eq!(
            redaction_target(&format!("/{field}/anything")),
            None,
            "/{field}/anything"
        );
    }
}

#[test]
fn redaction_target_refuses_the_exact_reported_panic_inputs() {
    // A nested pointer into `permissions` strips a field carrying no serde default,
    // leaving a value that cannot deserialize as a `PermissionSet`.
    for pointer in [
        "/permissions",
        "/permissions/default",
        "/permissions/users",
        "/permissions/property_overrides",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer}");
    }
}

#[test]
fn redaction_target_refuses_malformed_and_unknown_pointers() {
    for pointer in ["", "/", "system/hp", "/unknown", "/systemx", "/nameless"] {
        assert_eq!(redaction_target(pointer), None, "{pointer:?}");
    }
}

#[test]
fn name_is_a_leaf_band_with_no_interior() {
    // `/name` is a display string, not a container — mirrors the same rule in
    // `required_cap_for_path`.
    assert_eq!(redaction_target("/name"), Some(RedactionTarget::Band));
    assert_eq!(redaction_target("/name/first"), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib data::permission::tests::redaction_target`
Expected: FAIL — `cannot find function redaction_target in this scope`.

- [ ] **Step 3: Write the implementation**

Insert into `src/server/src/data/permission.rs`, immediately after `required_cap_for_path`:

```rust
/// The four CONTENT bands of a `Document`. Redaction operates on these and never on
/// the structural envelope (`id`, `scope`, `doc_type`, `schema_version`, `source`,
/// `owner`, `permissions`, `parent_id`, `embedded`, `created_at`, `updated_at`), whose
/// fields are either required or carry access-control meaning. Exactly the set
/// `required_cap_for_path` maps to `cap::WRITE_FIELDS`.
pub const REDACTABLE_BANDS: [&str; 4] = ["name", "engine", "system", "base"];

/// What a `property_overrides` pointer targets, and therefore how egress removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionTarget {
    /// A whole band. Nulled in place: dropping the key would fail re-deserialization
    /// for a required field, and for an `Option` field would be indistinguishable from
    /// a document that never carried one, breaking the client's stable envelope shape.
    Band,
    /// A path inside a band, landing in an untyped `serde_json::Value` or an `Option`.
    /// Removed with a pointer strip, where callers rely on true key absence.
    Within,
}

/// Classify a `property_overrides` pointer, or `None` when nothing may redact it.
///
/// INVARIANT: a `Within` result guarantees the STRIP lands in untyped or optional data,
/// never on a required field — that is what makes it provably non-destructive to
/// deserialization. A `Band` result carries no such guarantee and does not need one: it
/// nulls the field in place, which is precisely why `system` (required, not an `Option`)
/// is handled that way rather than stripped.
///
/// Both properties are what the ingress gate and the egress filter must agree on. They
/// agree by reading THIS function; two independent implementations verified to agree by
/// inspection is what drifted into a panic.
///
/// `/name` is a leaf: `/name/...` has no sub-path and classifies as `None`, mirroring
/// `required_cap_for_path`.
/// # Examples
///
/// ```
/// use shadowcat::data::permission::{redaction_target, RedactionTarget};
///
/// assert_eq!(redaction_target("/system"), Some(RedactionTarget::Band));
/// assert_eq!(redaction_target("/system/hp"), Some(RedactionTarget::Within));
/// assert_eq!(redaction_target("/permissions/default"), None);
/// ```
pub fn redaction_target(pointer: &str) -> Option<RedactionTarget> {
    let rest = pointer.strip_prefix('/')?;
    for band in REDACTABLE_BANDS {
        if rest == band {
            return Some(RedactionTarget::Band);
        }
        // `/name` carries no interior; every other band is a container.
        if band != "name" {
            if let Some(inner) = rest.strip_prefix(band) {
                if let Some(tail) = inner.strip_prefix('/') {
                    if !tail.is_empty() {
                        return Some(RedactionTarget::Within);
                    }
                }
            }
        }
    }
    None
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test --lib data::permission::tests::redaction_target && cargo test --lib data::permission::tests::name_is_a_leaf`
Expected: PASS, 6 tests.

- [ ] **Step 5: Prove the band list is load-bearing (mutation check)**

A parity test that passes because both paths are wrong the same way proves nothing, so
verify the test suite actually depends on the constant.

Temporarily edit `REDACTABLE_BANDS` to `["name", "engine", "system", "base"]` → `["name", "engine", "system", "unused"]`, then run:

Run: `cd src/server && cargo test --lib data::permission`
Expected: FAIL, in **three** distinct places — count them rather than accepting an aggregate
red. (1) `redaction_target_classifies_each_whole_band`'s loop fails on `/base`, which no longer
classifies. (2) That same test's `assert_eq!` on `REDACTABLE_BANDS` fails, naming the changed
list. (3) `redaction_target_classifies_within_a_band` fails on `/base/system/hp`.

If you see fewer than three, the suite is weaker than it looks and the mutation check has not
done its job — stop and report rather than proceeding on an aggregate FAIL.

**Revert the edit and re-run to confirm green before proceeding.** Confirm the revert
landed by diffing — a mutation that never took effect and a test that does not gate
produce identical output.

- [ ] **Step 6: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/data/permission.rs
git commit -m "feat(permission): classify redaction pointers by content band

One shared classifier decides what a property_overrides pointer means, so
ingress and egress stop duplicating the judgement. A Some result guarantees
the removal lands in untyped or optional data.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/data/permission.rs
```

---

### Task 2: Ingress rejects an unclassifiable override pointer

**Files:**
- Modify: `src/server/src/data/validation.rs` (`validate_property_overrides`)
- Test: `src/server/src/data/validation.rs` (`mod tests`)

**Interfaces:**
- Consumes: `crate::data::permission::redaction_target` from Task 1.
- Produces: no new symbols. `validate_property_overrides` keeps its signature
  `pub fn validate_property_overrides(doc: &Document) -> Result<(), DataError>` and its two
  existing call sites in `SqliteRepository::apply_intent` (the Create and Update branches)
  need **no edit** — they already call it.

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `src/server/src/data/validation.rs`. The helper
`doc_with_system` already exists there; build on it.

```rust
fn doc_with_override(pointer: &str) -> Document {
    let mut d = doc_with_system(serde_json::json!({ "hp": 1 }));
    d.permissions.property_overrides.insert(
        pointer.to_string(),
        crate::data::document::Visibility::GmOnly,
    );
    d
}

#[test]
fn override_naming_an_envelope_field_is_rejected() {
    for pointer in [
        "/permissions",
        "/permissions/default",
        "/permissions/users",
        "/permissions/property_overrides",
        "/owner",
        "/id",
        "/scope",
        "/doc_type",
        "/schema_version",
        "/source",
        "/parent_id",
        "/embedded",
        "/embedded/items/0",
        "/created_at",
        "/updated_at",
    ] {
        assert!(
            matches!(
                validate_property_overrides(&doc_with_override(pointer)),
                Err(DataError::BadPath(_))
            ),
            "{pointer} must be rejected at ingress"
        );
    }
}

#[test]
fn override_naming_a_content_band_is_accepted() {
    for pointer in [
        "/name",
        "/engine",
        "/engine/vision",
        "/system",
        "/system/hp",
        "/system/a/b/c",
        "/base",
        "/base/system/hp",
    ] {
        assert!(
            validate_property_overrides(&doc_with_override(pointer)).is_ok(),
            "{pointer} must be accepted at ingress"
        );
    }
}

#[test]
fn an_embedded_child_override_is_classified_too() {
    let mut parent = doc_with_system(serde_json::json!({}));
    let child = doc_with_override("/permissions/default");
    parent
        .embedded
        .insert("items".to_string(), vec![child]);
    assert!(matches!(
        validate_property_overrides(&parent),
        Err(DataError::BadPath(_))
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib data::validation::tests::override_naming`
Expected: FAIL — `override_naming_an_envelope_field_is_rejected` returns `Ok` today, because
well-formedness is the only check.

- [ ] **Step 3: Write the implementation**

Replace the loop body in `validate_property_overrides`:

```rust
pub fn validate_property_overrides(doc: &Document) -> Result<(), DataError> {
    for key in doc.permissions.property_overrides.keys() {
        if key.is_empty() || !key.starts_with('/') || key.ends_with('/') {
            return Err(DataError::BadPath(key.clone()));
        }
        // Redaction operates on content bands, never on the structural envelope. An
        // unclassifiable pointer is refused here so no stored override can later ask
        // egress to remove a field it must not touch.
        if crate::data::permission::redaction_target(key).is_none() {
            return Err(DataError::BadPath(key.clone()));
        }
    }
    for children in doc.embedded.values() {
        for child in children {
            validate_property_overrides(child)?;
        }
    }
    Ok(())
}
```

Update this function's doc comment so it states the band rule alongside the existing
well-formedness rule. Do not leave the old comment describing only the trailing-slash case —
treat every comment on a line you touch as stale until verified.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test --lib data::validation`
Expected: PASS, including the four pre-existing `validate_property_overrides` tests.

- [ ] **Step 5: Confirm no repo-wide override key is newly rejected**

Every override key constructed anywhere — server, client, and tests — must already be inside
the band whitelist. Verify rather than assume:

Run: `git grep -n "property_overrides" -- src | grep -v "^src/server/src/data/validation.rs"`
Expected: every literal pointer key you find is `/name`, `/engine`, `/engine/...`, `/system`,
`/system/...`, or `/base`. **If any key falls outside the whitelist, STOP and report it** —
that is a scope question, not something to whitelist around.

- [ ] **Step 6: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/data/validation.rs
git commit -m "fix(validation): reject a property override naming the envelope

An override pointer that redaction cannot classify is refused at both Create
and Update ingress, so no stored key can ask egress to remove a structural
field. Closes the reachable half of the redaction defect.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/data/validation.rs
```

---

### Task 3: Redaction fails closed instead of panicking

**Files:**
- Modify: `src/server/src/data/permission.rs` (`filter_properties`, `collect_hidden`, `filter_command`)
- Modify: `src/server/src/http/routes.rs` (`list_documents` ~line 975, `get_document` ~line 1026)
- Modify: `src/server/src/data/search.rs` (the `index_content` call ~line 116)
- Modify: `src/server/src/data/sqlite.rs` (the `SearchHit` construction ~line 2789)
- Test: `src/server/src/data/permission.rs` (`mod tests`)

**Interfaces:**
- Consumes: `redaction_target`, `RedactionTarget` from Task 1.
- Produces:
  - `pub struct RedactionError { pub pointer: String }` with `Display` + `std::error::Error`
  - `pub fn filter_properties(doc: &Document, access: &Access) -> Result<Document, RedactionError>`
  - `filter_command` keeps its existing signature and return type.

**Per-caller fail-closed policy.** This is the task's real content; the signature change is
mechanical. A secrecy gate that meets an input it cannot classify must **withhold** — never panic,
never guess:

| Caller | Policy on `Err` |
|---|---|
| `filter_command` Create / Delete arms | drop the op for that recipient (`continue`) |
| `filter_command` Update arm | drop the op for that recipient (`continue`) |
| `list_documents` | omit that document from the list, log at `warn` |
| `get_document` | return an error response — a single-document read has nothing to withhold *into* |
| `search` hit construction (`sqlite.rs`) | omit that hit, log at `warn` |
| `index_content` (`search.rs`) | index **empty** public content for that document, log at `warn`. This is the write path: never fail the write, and never index unredacted text |

> **Deviation from the spec's §5 phrasing, flagged for the reviewer.** The spec said
> `get_document`/`search` "error rather than ship a half-redacted document". This plan refines
> that: collection-returning callers (`list_documents`, `search`) **omit the item**, and only the
> single-document caller errors. Both are the same fail-closed direction — withhold — and omission
> keeps one poisoned document from denying an entire list or search to every reader, which would
> recreate a denial of service in the fix for one.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/server/src/data/permission.rs`. The `doc(perms, system)` helper
already exists there.

```rust
/// Build a `PermissionSet` carrying one override at `pointer`, hidden from non-GMs.
fn perms_with_override(pointer: &str) -> PermissionSet {
    let mut p = PermissionSet {
        default: crate::data::document::DocRole::Observer,
        ..Default::default()
    };
    p.property_overrides.insert(
        pointer.to_string(),
        crate::data::document::Visibility::GmOnly,
    );
    p
}

fn non_gm() -> Access {
    Access {
        caps: Default::default(),
        all: false,
        see_gm_only: false,
        is_owner: false,
    }
}

#[test]
fn filter_properties_errors_instead_of_panicking_on_a_nested_permissions_override() {
    // The exact reported input: a nested `/permissions/...` override strips a
    // `PermissionSet` field carrying no serde default, so the value cannot re-deserialize.
    let d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    let err = filter_properties(&d, &non_gm()).expect_err("must not deserialize");
    assert_eq!(err.pointer, "/permissions/default");
}

#[test]
fn filter_properties_errors_on_a_whole_permissions_override() {
    // A whole `/permissions` override is refused as unclassifiable rather than
    // substituting the fail-closed default permission set for the real one: that
    // substitution does not panic, it ships a wrong document.
    let d = doc(
        perms_with_override("/permissions"),
        serde_json::json!({ "hp": 1 }),
    );
    assert!(filter_properties(&d, &non_gm()).is_err());
}

#[test]
fn filter_properties_still_redacts_every_content_band() {
    for (pointer, check) in [
        ("/system/secret", "system"),
        ("/engine", "engine"),
        ("/name", "name"),
    ] {
        let d = doc(
            perms_with_override(pointer),
            serde_json::json!({ "secret": "MOCK_SECRET_A", "public": 1 }),
        );
        let out = filter_properties(&d, &non_gm())
            .unwrap_or_else(|e| panic!("{pointer} must still redact cleanly: {e}"));
        match check {
            "system" => {
                assert!(out.system.get("secret").is_none());
                assert_eq!(out.system["public"], 1);
            }
            "engine" => assert!(out.engine.is_none()),
            "name" => assert!(out.name.is_none()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn a_gm_recipient_is_unaffected_by_an_unclassifiable_override() {
    // The GM short-circuit returns before any classification runs, so a GM never
    // loses a document to a poisoned override.
    let d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    let gm = Access {
        caps: Default::default(),
        all: true,
        see_gm_only: true,
        is_owner: false,
    };
    assert!(filter_properties(&d, &gm).is_ok());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib data::permission::tests::filter_properties_errors`
Expected: FAIL — the first test **panics** with `filtered document deserializes` rather than
returning an error. That panic is the defect under repair; seeing it here is the proof the test
is discriminating.

- [ ] **Step 3: Add the error type and convert `filter_properties`**

In `src/server/src/data/permission.rs`, add above `filter_properties`:

```rust
/// A redaction input the classifier could not place in a content band. Egress
/// withholds rather than guessing: the alternatives are shipping a document whose
/// structural envelope was silently rewritten, or panicking the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionError {
    /// The pointer that could not be classified.
    pub pointer: String,
}

impl std::fmt::Display for RedactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unclassifiable redaction pointer {}", self.pointer)
    }
}

impl std::error::Error for RedactionError {}
```

Then change `filter_properties`' signature to
`pub fn filter_properties(doc: &Document, access: &Access) -> Result<Document, RedactionError>`
and rewrite its body's tail. The early GM return becomes `return Ok(out);`, the embedded
recursion must propagate (`filter_properties(&c, access)` now returns a `Result`, so collect
into a `Result<Vec<_>, _>` and `?` it), and the strip loop becomes:

```rust
    let mut whole = serde_json::to_value(&out).expect("document serializes");
    for pointer in hidden {
        match redaction_target(&pointer) {
            Some(RedactionTarget::Band) => {
                if let Some(f) = whole.get_mut(&pointer[1..]) {
                    *f = serde_json::Value::Null;
                }
            }
            Some(RedactionTarget::Within) => strip_pointer(&mut whole, &pointer),
            None => return Err(RedactionError { pointer }),
        }
    }
    serde_json::from_value(whole).map_err(|_| RedactionError {
        pointer: "<document>".to_string(),
    })
```

Update the doc comment and its doctest: the doctest currently ends
`let filtered = filter_properties(&doc, &observer);` and must become
`let filtered = filter_properties(&doc, &observer).unwrap();`. Rustdoc doctests are
CI-blocking, so a stale doctest fails the build.

**Do not delete the `to_value` `expect`** — serializing a well-formed `Document` cannot fail,
and that assertion is not the one this task removes. The two removed are the `from_value`
`expect` here and, if present after the rewrite, any second one on the same path.

- [ ] **Step 4: Propagate through `collect_hidden` and `filter_command`**

`collect_hidden` produces prefixed pointers for the delta path (a child at `embedded[key][i]`
contributes `/embedded/<key>/<i>{pointer}`), so classify the **unprefixed** override key before
the prefix is applied — that is what stops the change-delta path diverging from whole-document
egress. Change its signature to return `Result<(), RedactionError>` and, inside the
`property_overrides` loop, return `Err` when `redaction_target(p)` is `None`.

In `filter_command`, apply the policy table above. The Create arm becomes:

```rust
                if access.has(cap::READ) {
                    // Withhold rather than guess: a recipient who cannot be given a
                    // correctly redacted document is given none.
                    let Ok(filtered) = filter_properties(doc, &access) else {
                        tracing::warn!(doc_id = %doc.id, "redaction failed; dropping op for recipient");
                        continue;
                    };
                    out_ops.push(Operation::Create { doc: filtered });
                }
```

Apply the identical shape to the Delete arm. In the Update arm, `collect_hidden(cur, &access, "", &mut hidden)`
becomes fallible — drop the op the same way on `Err`.

- [ ] **Step 5: Update the four out-of-file call sites**

`src/server/src/http/routes.rs`, `list_documents` — replace the `filter_map` closure tail:

```rust
            if !access.has(cap::READ) {
                return None;
            }
            match filter_properties(&d, &access) {
                Ok(filtered) => Some(filtered),
                Err(e) => {
                    tracing::warn!(doc_id = %d.id, error = %e, "omitting document from list");
                    None
                }
            }
```

`src/server/src/http/routes.rs`, `get_document` — replace the final line:

```rust
    let filtered = filter_properties(&doc, &access).map_err(|e| {
        tracing::warn!(doc_id = %doc.id, error = %e, "refusing to serve a document redaction cannot classify");
        AppError::Internal
    })?;
    Ok(Json(filtered))
```

Use whatever the crate's existing 500-class `AppError` variant is named — read the `AppError`
enum in that file rather than assuming `Internal`, and use the real variant.

`src/server/src/data/sqlite.rs`, the search-hit construction — replace the `hits.push(...)`:

```rust
                let document = match crate::data::permission::filter_properties(&doc, &access) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "omitting search hit");
                        continue;
                    }
                };
                hits.push(SearchHit {
                    document,
                    score: row.get("score"),
                    snippet: row.get("snippet"),
                });
```

`src/server/src/data/search.rs`, the index build — this is the **write** path, so it must not
fail the write, and must not index unredacted text:

```rust
    match crate::data::permission::filter_properties(doc, &non_gm) {
        Ok(redacted) => index_content(&redacted),
        Err(e) => {
            tracing::warn!(doc_id = %doc.id, error = %e, "indexing empty public content");
            String::new()
        }
    }
```

Match the surrounding function's actual return type and binding names — read the function before
editing; the snippet above shows the shape, not a literal drop-in.

- [ ] **Step 6: Run the tests to verify they pass**

**Expect roughly two dozen compile errors on the first run, and do not read them as a design
problem.** Two distinct families of pre-existing test call this function directly: the
`filter_command_*` family, and a larger set that indexes straight into the returned document
(`filter_properties(&d, &a_owner).system["name"]` and similar — `owner_or_gm_visible_to_owner_and_gm_not_other_player`,
`owner_cannot_see_gm_only`, `embedded_owner_or_gm_redacted_for_non_owner`, and more). **Every**
one needs a trailing `.unwrap()`. That is expected mechanical fallout from the signature change.

Run: `cd src/server && cargo test --lib data::permission`
Expected: PASS, including every pre-existing test in both families. Those tests are the real
regression net for this change — if any of them needed editing **beyond** adding `.unwrap()` or
`?`, **stop and report it**: a behavior change in a redaction test is not a mechanical fixup.

- [ ] **Step 7: Prove the fail-closed path is reachable end to end**

The unit tests above call `filter_properties` directly. Add one test proving a poisoned document
is withheld through `filter_command`, which is the per-recipient broadcast egress path:

```rust
#[tokio::test]
async fn filter_command_drops_a_create_whose_redaction_cannot_be_classified() {
    // Mirror the construction the neighbouring `filter_command_*` tests use for their
    // `Command`, `PermissionContext` and `WorldCapDefaults` fixtures.
    let d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    // ... build `cmd` with a single Create of `d`, a non-GM `ctx`, empty defaults ...
    let out = filter_command(&cmd, &ctx, &world_defaults, &current, |_| None);
    assert!(
        out.ops.is_empty(),
        "the op must be withheld, not shipped half-redacted"
    );
    assert_eq!(out.seq, cmd.seq, "seq is preserved so the sequence guard sees no gap");
}
```

Copy the fixture construction from the nearest existing `filter_command_*` test in the same
module rather than inventing one.

- [ ] **Step 8: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all PASS, including doctests.

- [ ] **Step 9: Commit**

```bash
git add src/server/src/data/permission.rs src/server/src/http/routes.rs src/server/src/data/search.rs src/server/src/data/sqlite.rs
git commit -m "fix(permission): redaction fails closed instead of panicking

filter_properties returns a Result and both deserialize assertions are gone.
Every caller withholds on failure: broadcast drops the op for that recipient,
list and search omit the item, a single-document read errors, and the index
writes empty public content rather than unredacted text.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/data/permission.rs src/server/src/http/routes.rs src/server/src/data/search.rs src/server/src/data/sqlite.rs
```

---

### Task 4: The field-change validator requires its value keys

**Ledger id:** TD26.

**Files:**
- Modify: `src/client/core/src/wire.ts` (`WireFieldChange`, `fieldChangeSchemaImpl`)
- Test: `src/client/core/src/wire.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `FieldChangeSchema` rejects a frame omitting `old` or `new`; `WireFieldChange`'s
  `old`/`new` become required properties of type `unknown`.

**Why this is not cosmetic.** `z.unknown()` accepts an *absent* key, because `undefined`
satisfies `unknown` — so `FieldChangeSchema.safeParse({ path })` succeeds today with both value
keys gone. The Rust source never omits them (only `remove` carries a skip-serializing attribute),
so a frame lacking them is malformed and the client's validation boundary currently admits it.
The correct shape is a required key that still permits an explicit `null` value, since `old` is
genuinely absent-valued for a key that did not previously exist.

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/wire.test.ts`:

```ts
describe("FieldChangeSchema", () => {
  it("rejects a frame that omits the pre-image key", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp", new: 3 }).success).toBe(false);
  });

  it("rejects a frame that omits the new-value key", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp", old: 1 }).success).toBe(false);
  });

  it("rejects a frame carrying only a path", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp" }).success).toBe(false);
  });

  it("accepts an explicit null pre-image, which is a real value for a new key", () => {
    expect(
      FieldChangeSchema.safeParse({ path: "/system/hp", old: null, new: 3 }).success,
    ).toBe(true);
  });

  it("accepts a removal, where new is conventionally null", () => {
    expect(
      FieldChangeSchema.safeParse({ path: "/system/hp", old: 1, new: null, remove: true })
        .success,
    ).toBe(true);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- wire.test.ts`
Expected: the first three FAIL (`expected true to be false`); the last two already pass.

- [ ] **Step 3: Write the implementation**

In `src/client/core/src/wire.ts`, replace `z.unknown()` on both value keys with a schema that
requires the key while accepting any value including `null` and `undefined`:

```ts
// Unannotated impl const — see the module-level note above the `z` import.
export const fieldChangeSchemaImpl = z.object({
  path: z.string(),
  // `z.unknown()` alone would accept an ABSENT key, because `undefined` satisfies
  // `unknown`. The Rust source never omits either value key, so a frame lacking one is
  // malformed. `.refine` on the whole object is what sees absence — a per-key schema
  // cannot distinguish "absent" from "present and undefined".
  old: z.unknown(),
  new: z.unknown(),
  remove: z.boolean().optional(),
})
  .refine((v) => "old" in v, { message: "field change must carry an `old` pre-image", path: ["old"] })
  .refine((v) => "new" in v, { message: "field change must carry a `new` value", path: ["new"] });
```

Then update `WireFieldChange` so `old` and `new` are **required** properties (drop the `?` and
the doc-comment sentence explaining why they were typed optional — that sentence describes the
laxity this task removes, and leaving it makes the comment false).

> If the `z.ZodType<WireFieldChange>` annotation on `FieldChangeSchema` no longer accepts the
> refined schema (a `ZodEffects` is not a `ZodObject`), keep the annotation on the exported
> const and let the impl const stay unannotated — the file already establishes that convention.
> If the annotation cannot be satisfied at all, **stop and report**; do not reach for a
> `@ts-ignore`, which is a forbidden suppression.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- wire.test.ts`
Expected: PASS.

- [ ] **Step 5: Run the full repo gate**

A wire-schema change can break sibling packages' fixtures in ways a typecheck does not see, so
the filtered run is not sufficient.

Run: `pnpm -r test && pnpm -r typecheck && pnpm lint`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "fix(wire): require the value keys on an inbound field change

z.unknown() accepts an absent key, so a frame carrying only a path parsed
clean. The Rust source never omits either value, so such a frame is malformed.
An explicit null value stays valid — that is a real pre-image for a new key.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/client/core/src/wire.ts src/client/core/src/wire.test.ts
```

---

### Task 5: Narrow the role-keyed grant map and land the snippet exposure note

**Ledger ids:** TD31, TD27.

**Files:**
- Modify: `src/client/core/src/wire.ts` (`WireCapabilityGrants`, `capabilityGrantsSchemaImpl`,
  `WireSearchHit`)
- Test: `src/client/core/src/wire.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `CapabilityGrantsSchema` rejects an unknown role key. `by_user` is **unchanged** —
  its keys are user ids, which are genuinely open.

- [ ] **Step 1: Write the failing tests**

```ts
describe("CapabilityGrantsSchema", () => {
  it("accepts the three document roles as grant keys", () => {
    expect(
      CapabilityGrantsSchema.safeParse({
        by_role: { owner: ["core:manage_embedded"], observer: [], none: [] },
        by_user: {},
      }).success,
    ).toBe(true);
  });

  it("accepts a partial role map, matching a Rust map that omits roles", () => {
    expect(
      CapabilityGrantsSchema.safeParse({ by_role: { owner: [] }, by_user: {} }).success,
    ).toBe(true);
  });

  it("rejects an unknown role key", () => {
    expect(
      CapabilityGrantsSchema.safeParse({ by_role: { admin: [] }, by_user: {} }).success,
    ).toBe(false);
  });

  it("still accepts an arbitrary user id as a by_user key", () => {
    expect(
      CapabilityGrantsSchema.safeParse({
        by_role: {},
        by_user: { "00000000-0000-0000-0000-000000000001": ["core:delete"] },
      }).success,
    ).toBe(true);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- wire.test.ts`
Expected: "rejects an unknown role key" FAILS; the other three pass.

- [ ] **Step 3: Write the implementation**

In `src/client/core/src/wire.ts`:

```ts
export type WireCapabilityGrants = {
  /** Extra capabilities granted to everyone holding a given `DocRole`, keyed by role. The
   * Rust source is a map that may omit any role, so every key is optional. */
  by_role: Partial<Record<z.infer<typeof DocRoleSchema>, string[]>>;
  /** Extra capabilities granted to specific users (by id), regardless of role. User ids are
   * genuinely open, so this map stays string-keyed. */
  by_user: Record<string, string[]>;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const capabilityGrantsSchemaImpl = z.object({
  by_role: z.record(DocRoleSchema, z.array(z.string())),
  by_user: z.record(z.array(z.string())),
});
```

`DocRoleSchema` is declared earlier in the same file as `z.enum(["owner", "observer", "none"])`,
so it is already in scope. Zod v3's `ZodRecord` parses each key with the key schema, which is
what makes an unknown role fail.

> Zod v3 infers a **non-partial** `Record` from an enum-keyed record, so the
> `z.ZodType<WireCapabilityGrants>` annotation may or may not accept it. If it errors, keep the
> exported annotation and leave the impl const unannotated, per the file's existing convention.
> Do **not** widen the declared type back to `Record<string, string[]>` to make the annotation
> compile — that reinstates the defect. If neither shape works, stop and report.

For TD27, replace the `snippet` doc comment on `WireSearchHit`:

```ts
  /** Highlighted match snippet from the recipient's own index partition. Every `engine` AND
   * `system` string leaf, plus the document's `name`, is swept into the full-text index and
   * can surface here and in `document` — `index_content` walks all three — so a consumer must
   * render this as inert text and never as innerHTML. */
  snippet: string;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- wire.test.ts`
Expected: PASS.

- [ ] **Step 5: Run the full repo gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm lint:docs && pnpm lint:props && pnpm lint:comments`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "fix(wire): key the role grant map by DocRole; note snippet exposure

by_role admitted any string, so a frame naming an unknown role parsed clean.
by_user stays open — its keys are user ids. The search snippet now carries the
inert-text handling constraint its Rust source states.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/client/core/src/wire.ts src/client/core/src/wire.test.ts
```

---

### Task 6: Record the replay-redaction ruling

**Ledger id:** PW19.

**This task is BLOCKED until the user rules.** The user has deferred the ruling to the plan
buddy-check: two blind reviewers argue the leak question independently, and their convergence or
stalemate goes to the user, who then authorizes a branch. Both branches are specified here so the
executing agent needs no further input once one is recorded in its task brief. **An agent that
finds no branch recorded stops and asks — it does not choose.**

**Background.** `filter_command`'s Update arm loads each op's document to resolve visibility, so on
resync or replay a property whose visibility was flipped after the event is redacted under the
*new* policy, not the policy in force at that sequence.

**Branch A — accept, with the reasoning recorded in code (the recommendation).**

The analysis: a field that was hidden and is now visible is public anyway, and one that was
visible and is now hidden is over-redacted. Neither direction leaks. So the only thing at stake is
audit-grade replay fidelity, which nothing consumes.

- [ ] **Step A1: Extend `filter_command`'s doc comment**

Add to the existing doc comment on `filter_command`, in the project's present-tense
constraint style — no history narration, no document pointers:

```rust
/// CONSTRAINT: an `Update`'s visibility is resolved against the document's CURRENT
/// permission set, not the set in force at that seq. Replay is recovery, not audit, and
/// neither direction leaks: a field since made visible is public anyway, and one since
/// hidden is over-redacted. Point-in-time fidelity would require snapshotting the
/// relevant permissions into the event, and nothing consumes an audit-grade replay.
```

- [ ] **Step A2: Update the tracking docs**

Change that entry's status in `docs/POST_WORK_FINDINGS.md` from "Needs triage" to accepted,
stating the no-leak-either-direction reasoning and that the constraint now lives on the symbol.

- [ ] **Step A3: Run the gate and commit**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

```bash
git add src/server/src/data/permission.rs docs/POST_WORK_FINDINGS.md
git commit -m "docs(permission): state the replay-redaction constraint on the symbol

Replayed history resolves visibility against the current permission set.
Neither direction leaks, so the behavior stands and the constraint is now
recorded where a maintainer reads it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/data/permission.rs docs/POST_WORK_FINDINGS.md
```

**Branch B — implement point-in-time redaction.**

If the user rules that replay must be point-in-time faithful, this is **not** a step in this task:
it changes the event record's shape (the relevant permissions must be snapshotted into the event
or attached to the broadcast), which touches the command representation, the event log, and
resync. Open it as its own ledger item, assign it to Phase 1 as a new task, and re-plan that task
before writing code. Do not attempt it inline.

---

### Task 7: Phase closeout

**Files:**
- Modify: `docs/OPEN_BUGS.md`, `docs/CLOSED_BUGS.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`
- Modify: `docs/superpowers/specs/2026-08-13-debt-burndown-campaign-design.md` (ledger dispositions)
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`
- Modify: `.claude/.claude-plugin/plugin.json`

- [ ] **Step 1: Move the closed bug**

Remove the `property_overrides` entry from `docs/OPEN_BUGS.md` and add its resolution to
`docs/CLOSED_BUGS.md`, citing the symbols (`redaction_target`, `REDACTABLE_BANDS`,
`RedactionError`) and never a file name or line number.

- [ ] **Step 2: Close the to-do entries**

Remove the TD26, TD27 and TD31 entries from `docs/TODO.md`. Record each disposition in the
spec's ledger.

- [ ] **Step 3: Record every per-item disposition**

In the spec's §4 tables, mark OB2, TD26, TD27, TD31 and PW19 with what was done and the
evidence. **Every id assigned to Phase 1 must have a line.** Append any newly discovered item
as `NEW-n` with its phase assignment, per the spec's §2.4.

- [ ] **Step 4: Update the subsystem skill**

`shadowcat-codebase-documents-permissions` gains the band model as a Hard Invariant: redaction
operates on content bands, never the envelope; ingress and egress read one classifier; egress
withholds on an unclassifiable input. This is the reviewed skill-update gate — dispatch
`shadowcat-spec-reviewer` to confirm the skill diff accurately captures the change, with no
omission, drift, or broken pointer.

- [ ] **Step 5: Bump the plugin version**

Increment `version` in `.claude/.claude-plugin/plugin.json`. A directory-sourced plugin serves a
cached snapshot, so without the bump the skill edit reaches no consuming repo and a stale copy is
indistinguishable from a current one. Report to the user that
`claude plugin marketplace update shadowcat` then `claude plugin update shadowcat-codebase` must
be run in each consuming repo — that is a shell action outside this task.

- [ ] **Step 6: Refresh the knowledge graph**

Run: `graphify update .`

- [ ] **Step 7: Run the full gate on both toolchains**

Run: `pnpm build && pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm lint:allowances`
Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all PASS. The client build must precede any cargo build — the embed validates `dist/`
at compile time.

- [ ] **Step 8: Commit and merge**

```bash
git add docs .claude
git commit -m "docs(phase1): sync trackers, skill, and plugin version

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- docs .claude
git checkout main
git merge --no-ff phase1-server-data-permissions-wire
```

Merge only after the whole-branch review returns clean. Do not push — the push gate is the full
sub-project.

---

## Self-Review

**Spec coverage.** OB2 → Tasks 1, 2, 3 (all three parts of the decided fix shape: shared
classifier, ingress rejection, `Result`-returning egress). TD26 → Task 4. TD31, TD27 → Task 5.
PW19 → Task 6. The spec's required tests for OB2 are all present: per-pointer ingress rejection
for each envelope field (Task 2 Step 1), acceptance for the four bands and their nested forms
(Task 2 Step 1), a regression test that the exact nested-permissions input errors instead of
panicking (Task 3 Step 1), and the mutation check that removing a band fails the suite (Task 1
Step 5). Phase closeout obligations from spec §5 → Task 7. No gap found.

**Placeholder scan.** No "TBD", no "add appropriate error handling", no "similar to Task N". Three
places deliberately instruct the implementer to read surrounding code rather than trust a literal:
the `AppError` variant name in Task 3 Step 5, the `index_content` binding names in the same step,
and the `filter_command` fixture construction in Task 3 Step 7. Each says so explicitly and gives
the shape — that is a verification instruction, not a placeholder.

**Type consistency.** `redaction_target` and `RedactionTarget` are named identically in Tasks 1, 2
and 3. `RedactionError`'s public field is `pointer` in its definition (Task 3 Step 3) and in the
test that asserts on it (Task 3 Step 1). `REDACTABLE_BANDS` is `[&str; 4]` in both its definition
and the test that iterates it. `filter_properties`' new signature is stated once in Task 3's
Interfaces block and used consistently at all five call sites.

**One deviation flagged, not absorbed:** Task 3's per-caller policy refines the spec's §5 phrasing
for `list_documents` and `search` from "error" to "omit the item and log". Both are fail-closed;
omission prevents one poisoned document from denying an entire list to every reader. Called out in
Task 3's policy table for the reviewer to accept or reject.
