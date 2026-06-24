# M10b — Factions + Name Privacy Implementation Plan

> **For agentic workers:** This plan is executed mainline (per the project's
> `mainline-plan-execution` directive): implement task-by-task in this session with an
> inline spec-compliance check per task and ONE buddy-check of the full branch before
> merge (see "Buddy-check directives"). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add world-configurable **factions** (a seeded, GM-editable registry config-document
driving a faction-colored token border + faction group-select) and day-one **name privacy**
(a new `OwnerOrGm` per-recipient visibility tier + a `displayName` fallback, fail-closed on
every egress path).

**Architecture:** Name privacy extends the M5 redaction infrastructure with a third
`Visibility` tier, `OwnerOrGm`, gated by a new `Access::is_owner` flag and a `can_see(tier)`
predicate — so a document's **owner** sees `OwnerOrGm` fields but never `GmOnly` ones. The
secret never reaches an unauthorized client on any fresh delivery, and a permission-tightening
Update **retroactively retracts** an already-delivered value (per-recipient `null` redaction) from
clients that can no longer see it; a single `actorDisplayName` accessor is the read chokepoint. Factions are a world-scoped singleton
config-document `doc_type:"faction-registry"` whose `system.factions` is an **id→faction map**
(a map, not an array, because `set_pointer` cannot grow arrays); a replaceable first-party
`module-factions` seeds three defaults idempotently and provides the GM editor. The token's
faction color rides a new `TokenNodeSpec.borderColor`; group-select uses a `TokenSelection`
holder on `AppContext` (mirroring `ActorSelection`) with a selection overlay drawn by the
select tool.

**Tech Stack:** Rust (axum/ts-rs/serde_json) server; TypeScript `@shadowcat/core`
(framework-neutral) + `@shadowcat/ui-kit` (Svelte 5 runtime) + `@shadowcat/render` (PixiJS);
Svelte 5 modules under `src/modules/*`. Tests: Vitest (`pnpm -r test`), cargo test (server).

## Global Constraints

- **Server structural-only (#6):** the server honors a **declared** JSON-pointer visibility
  tier; it never interprets `name`/faction semantics. Add NO actor/faction schema validation
  beyond the existing `system` size cap.
- **Fail-closed secrecy ([[fog-is-the-secrecy-gate-fail-closed]]):** the secret is the
  *absence* of the real name. A missing/garbled name yields the generic fallback, **never** a
  leak. The owner tier must never widen into `GmOnly`.
- **ts-rs sync (CI-enforced):** any Rust wire-type change regenerates `src/types/generated/*.ts`
  (run `cargo test` in `src/server`) AND is mirrored in the hand-written Zod schema in
  `src/client/core/src/wire.ts` (the `wire.test.ts` type-equivalence guard enforces parity).
- **Wire-shape single source of truth:** all document construction goes through builders in
  `src/client/core/src/scene-docs.ts` (never inline document literals in modules).
- **`set_pointer` cannot grow arrays:** any runtime-editable collection reached by an Update
  field-path must be an **object/map** (single-key set) or be replaced wholesale.
- **Center origin:** a token's `(x,y)` is its CENTER.
- **TDD, DRY, YAGNI, frequent commits.** Run `pnpm -r typecheck` before each client commit;
  `cargo fmt --check && cargo clippy -- -D warnings` before each server commit.

---

## Buddy-check directives

This checkpoint is **secrecy-critical** (a leaked real name is a security defect) and
cross-cutting (server tier + client accessor + render). Buddy-check the **full M10b branch**
(two independent reviewers, reconciled) before merge. Focus areas:

1. **`OwnerOrGm` never widens to `GmOnly`** — an owner (non-GM) sees `OwnerOrGm` fields but is
   still denied `GmOnly` fields on every path: whole-doc (`filter_properties`), update-delta
   (`filter_command` → `collect_hidden`/`redact_change`), embedded depth, search index, HTTP
   list/get. Verify `can_see` is the single predicate and no path bypasses it.
2. **Owner derivation at embedded depth** — embedded `OwnerOrGm` resolves against the
   **top-level** recipient's owner-status (the recursion carries the root recipient's
   `Access`); confirm this is intended and fail-closed (a non-owner non-GM sees nothing).
3. **Retroactive redaction is leak-free + per-recipient** (Task 2b) — tightening permissions
   nulls the now-hidden field for non-authorized recipients with `old:null` (the pre-image must
   not leak the real value); the owner keeps an `OwnerOrGm` field (not in their `collect_hidden`
   set) and the GM keeps everything. Confirm the `touches_permissions` trigger covers embedded
   permission changes and that the over-approximation only ever re-nulls already-hidden fields.
4. **Faction registry is a map + Updates are valid field-paths** — adds are single-key sets;
   removes replace the whole map; no array-growth Update exists.
5. **No leaked GM-only in the new tests** — fixtures use synthetic names only.

---

## Task 1: `OwnerOrGm` visibility tier — wire type end to end

**Files:**
- Modify: `src/server/src/data/document.rs:37-40` (`Visibility` enum)
- Regenerate: `src/types/generated/Visibility.ts` (via `cargo test`)
- Modify: `src/client/core/src/wire.ts:16` (`VisibilitySchema`)
- Verify: `src/client/core/src/wire.test.ts:26-30` (type-equivalence guard — no edit expected)

**Interfaces:**
- Produces: `Visibility` gains `OwnerOrGm` (serde `owner_or_gm`); client `Visibility` type +
  `VisibilitySchema` gain `"owner_or_gm"`.

- [ ] **Step 1: Add the Rust variant**

`src/server/src/data/document.rs` (the `Visibility` enum):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    All,
    GmOnly,
    /// Readable by the document's owner and the GM; redacted from everyone else.
    /// The recipient's owner-status is `Access::is_owner` (see permission.rs).
    OwnerOrGm,
}
```

- [ ] **Step 2: Regenerate bindings + confirm**

Run: `cd src/server && cargo test`
Expected: PASS; `src/types/generated/Visibility.ts` now reads
`export type Visibility = "all" | "gm_only" | "owner_or_gm";`

- [ ] **Step 3: Mirror in the client Zod schema**

`src/client/core/src/wire.ts:16`:
```ts
export const VisibilitySchema = z.enum(["all", "gm_only", "owner_or_gm"]);
```

- [ ] **Step 4: Verify the drift guard + typecheck pass**

Run: `pnpm --filter @shadowcat/core test wire && pnpm -r typecheck`
Expected: PASS (the `wire.test.ts` `Visibility` type-equivalence check holds; no edit needed).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/document.rs src/types/generated/Visibility.ts src/client/core/src/wire.ts
git commit -m "feat(m10b): add OwnerOrGm visibility tier (wire type)"
```

---

## Task 2: Owner-aware redaction (`Access::is_owner` + `can_see`)

**Files:**
- Modify: `src/server/src/data/permission.rs` (`Access` struct + `resolve_access` + `filter_properties`
  + `collect_gm_only`→`collect_hidden` + `filter_command` Update path; add tests)
- Modify: `src/server/src/data/search.rs:36-48` (`index_content_public` Access literal)
- Modify: any other `Access { … }` literal the compiler flags (test helpers)

**Interfaces:**
- Consumes: `Visibility::OwnerOrGm` (Task 1); `Document.owner: Option<Uuid>`.
- Produces:
  ```rust
  pub struct Access { pub caps: BTreeSet<String>, pub all: bool, pub see_gm_only: bool, pub is_owner: bool }
  impl Access { pub fn can_see(&self, v: Visibility) -> bool }
  ```
  `can_see(All)=true`; `can_see(GmOnly)=see_gm_only`; `can_see(OwnerOrGm)=see_gm_only || is_owner`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `src/server/src/data/permission.rs` (reuse the existing
`doc(..)`/access helpers there; build a non-GM `Access` via `resolve_access`):
```rust
#[test]
fn owner_or_gm_visible_to_owner_and_gm_not_other_player() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut d = doc(10);
    d.owner = Some(owner);
    d.system = serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" });
    d.permissions
        .property_overrides
        .insert("/system/name".into(), Visibility::OwnerOrGm);

    // Owner (non-GM observer default) sees the real name.
    let a_owner = resolve_access(owner, WorldRole::Player, &d);
    let v_owner = filter_properties(&d, &a_owner);
    assert_eq!(v_owner.system["name"], "Goblin Skirmisher");

    // Another player does NOT.
    let a_other = resolve_access(other, WorldRole::Player, &d);
    let v_other = filter_properties(&d, &a_other);
    assert!(v_other.system.get("name").is_none());
    assert_eq!(v_other.system["displayName"], "Goblin");

    // GM sees it.
    let a_gm = resolve_access(other, WorldRole::Gm, &d);
    assert_eq!(filter_properties(&d, &a_gm).system["name"], "Goblin Skirmisher");
}

#[test]
fn owner_cannot_see_gm_only() {
    let owner = Uuid::from_u128(1);
    let mut d = doc(11);
    d.owner = Some(owner);
    d.system = serde_json::json!({ "name": "PC", "secret": "GM note" });
    d.permissions.property_overrides.insert("/system/name".into(), Visibility::OwnerOrGm);
    d.permissions.property_overrides.insert("/system/secret".into(), Visibility::GmOnly);

    let a_owner = resolve_access(owner, WorldRole::Player, &d);
    let v = filter_properties(&d, &a_owner);
    assert_eq!(v.system["name"], "PC");          // owner sees OwnerOrGm
    assert!(v.system.get("secret").is_none());    // owner still denied GmOnly
}

#[test]
fn embedded_owner_or_gm_redacted_for_non_owner() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut child = doc(21);
    child.system = serde_json::json!({ "name": "Hidden", "displayName": "Thing" });
    child.permissions.property_overrides.insert("/system/name".into(), Visibility::OwnerOrGm);
    let mut parent = doc(20);
    parent.owner = Some(owner);
    parent.embedded.insert("actor".into(), vec![child]);

    let a_other = resolve_access(other, WorldRole::Player, &parent);
    let v = filter_properties(&parent, &a_other);
    assert!(v.embedded["actor"][0].system.get("name").is_none());

    let a_owner = resolve_access(owner, WorldRole::Player, &parent);
    let vo = filter_properties(&parent, &a_owner);
    assert_eq!(vo.embedded["actor"][0].system["name"], "Hidden");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd src/server && cargo test --lib owner`
Expected: FAIL to compile (`Access` has no `is_owner`; `resolve_access` literals incomplete).

- [ ] **Step 3: Add `is_owner` + `can_see`; set owner in `resolve_access`**

`src/server/src/data/permission.rs` — extend `Access`:
```rust
/// A user's effective capabilities on a document. `all` is the GM/admin short-circuit;
/// `see_gm_only` drives `GmOnly` redaction; `is_owner` additionally admits the `OwnerOrGm`
/// tier (a player still sees their own hidden PC's name) WITHOUT widening to `GmOnly`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub caps: BTreeSet<String>,
    pub all: bool,
    pub see_gm_only: bool,
    pub is_owner: bool,
}

impl Access {
    /// Whether the user holds capability `c` (GM holds everything).
    pub fn has(&self, c: &str) -> bool {
        self.all || self.caps.contains(c)
    }

    /// Whether a property declared at visibility tier `v` is readable by this recipient.
    /// `GmOnly` requires the GM short-circuit; `OwnerOrGm` also admits the document owner.
    pub fn can_see(&self, v: Visibility) -> bool {
        match v {
            Visibility::All => true,
            Visibility::GmOnly => self.see_gm_only,
            Visibility::OwnerOrGm => self.see_gm_only || self.is_owner,
        }
    }
}
```
In `resolve_access`, the GM branch and the non-GM tail both set `is_owner`:
```rust
    if world_role == WorldRole::Gm {
        return Access { caps: BTreeSet::new(), all: true, see_gm_only: true, is_owner: true };
    }
    // …existing role/caps resolution…
    Access {
        caps,
        all: false,
        see_gm_only: false,
        is_owner: doc.owner == Some(user),
    }
```

- [ ] **Step 4: Drive redaction through `can_see`**

In `filter_properties`, replace the `GmOnly`-equality strip predicate with `can_see` (the
`see_gm_only` early-return for GM stays — GM sees every tier):
```rust
    let hidden: Vec<String> = doc
        .permissions
        .property_overrides
        .iter()
        .filter(|(_, v)| !access.can_see(**v))
        .map(|(p, _)| p.clone())
        .collect();
    let mut whole = serde_json::to_value(&out).expect("document serializes");
    for pointer in hidden {
        strip_pointer(&mut whole, &pointer);
    }
```
Rename `collect_gm_only` → `collect_hidden`, taking the recipient `Access` so it collects every
tier the recipient cannot see (not just `GmOnly`):
```rust
/// Collect every property pointer in `doc` (and embedded descendants, parent-absolute) that
/// `access` may NOT see — `GmOnly` for any non-GM, `OwnerOrGm` for a non-owner non-GM. Lets
/// `Update`-delta redaction honor hidden fields at any depth, matching `filter_properties`.
fn collect_hidden(doc: &Document, access: &Access, prefix: &str, out: &mut Vec<String>) {
    for (p, v) in &doc.permissions.property_overrides {
        if !access.can_see(*v) {
            out.push(format!("{prefix}{p}"));
        }
    }
    for (key, children) in &doc.embedded {
        for (idx, child) in children.iter().enumerate() {
            collect_hidden(child, access, &format!("{prefix}/embedded/{key}/{idx}"), out);
        }
    }
}
```
In `filter_command`'s `Update` arm, update the call + variable name (the `see_gm_only`
short-circuit for GM stays; a non-GM owner falls through and collects only what *they* can't
see, so their `OwnerOrGm` changes pass):
```rust
                let kept: Vec<FieldChange> = if access.see_gm_only {
                    changes.clone()
                } else {
                    let mut hidden = Vec::new();
                    collect_hidden(&cur, &access, "", &mut hidden);
                    changes
                        .iter()
                        .filter_map(|ch| redact_change(ch, &hidden))
                        .collect()
                };
```

- [ ] **Step 5: Fix the `index_content_public` Access literal + any others the compiler flags**

`src/server/src/data/search.rs` (`index_content_public`) — the public (most-restrictive)
indexer is a non-owner non-GM:
```rust
    let non_gm = crate::data::permission::Access {
        caps: std::collections::BTreeSet::new(),
        all: false,
        see_gm_only: false,
        is_owner: false,
    };
```
Run `cargo build` and add `is_owner: <false unless the case is GM>` to every other flagged
`Access { … }` literal (test helpers). Grep to be exhaustive:
Run: `grep -rn "Access {" src/server/src` → confirm each literal now sets `is_owner`.

- [ ] **Step 6: Run the new + existing redaction tests**

Run: `cd src/server && cargo test`
Expected: PASS (new owner tests + all existing `gm_only`/`filter_command` tests unchanged).

- [ ] **Step 7: Lint + commit**

```bash
cd src/server && cargo fmt --check && cargo clippy -- -D warnings
cd ../.. && git add src/server/src/data/permission.rs src/server/src/data/search.rs
git commit -m "feat(m10b): owner-aware redaction (Access::is_owner + can_see)"
```

---

## Task 2b: Retroactive redaction on permission tightening

**Files:**
- Modify: `src/server/src/data/permission.rs` (`filter_command` Update arm + a `touches_permissions`
  helper; add tests)

**Interfaces:**
- Consumes: `collect_hidden`, `Access::can_see` (Task 2); `FieldChange`.
- Produces: when a broadcast `Update` tightens permissions, each recipient who can no longer see a
  field receives a synthesized `{ path, old: null, new: null }` retraction for that field, so a
  previously-delivered value cannot linger client-side. Per-recipient: authorized recipients
  (GM via `see_gm_only`; owner via `can_see`) get no retraction for fields they may still see.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `src/server/src/data/permission.rs`, mirroring the existing
`filter_command_*` harness (it builds an in-memory `Repository` with `cur` inserted, a
`PermissionContext` per recipient, and default `WorldCapDefaults`). The command under test is an
`Update` to `doc 30` whose change adds the name override; `cur` (post-apply) already carries both
the override and the real name:
```rust
#[tokio::test]
async fn permission_tightening_retracts_now_hidden_field_for_non_owner() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    // cur = post-apply doc: owner set, name present, /system/name now OwnerOrGm.
    let mut cur = doc(30);
    cur.owner = Some(owner);
    cur.system = serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" });
    cur.permissions.property_overrides.insert("/system/name".into(), Visibility::OwnerOrGm);
    let repo = /* in-memory repo with `cur` — mirror the existing filter_command tests */;

    let prev_overrides = serde_json::json!({});
    let new_overrides = serde_json::json!({ "/system/name": "owner_or_gm" });
    let cmd = Command {
        seq: 7, world_id: Uuid::from_u128(9), author: owner, ts: 0,
        ops: vec![Operation::Update {
            doc_id: cur.id,
            changes: vec![FieldChange { path: "/permissions/property_overrides".into(), old: prev_overrides, new: new_overrides }],
        }],
    };

    // Non-owner player: gets the permission change PLUS a null retraction of /system/name.
    let out = filter_command(&repo, &cmd, &ctx_for(other, WorldRole::Player), &WorldCapDefaults::default()).await;
    let Operation::Update { changes, .. } = &out.ops[0] else { panic!() };
    let retract = changes.iter().find(|c| c.path == "/system/name").expect("name retracted");
    assert_eq!(retract.new, serde_json::Value::Null);
    assert_eq!(retract.old, serde_json::Value::Null); // pre-image must not leak the real name

    // Owner: keeps the name (OwnerOrGm is visible to them) — no /system/name retraction.
    let out_owner = filter_command(&repo, &cmd, &ctx_for(owner, WorldRole::Player), &WorldCapDefaults::default()).await;
    let Operation::Update { changes, .. } = &out_owner.ops[0] else { panic!() };
    assert!(!changes.iter().any(|c| c.path == "/system/name"));

    // GM: sees everything; no synthesized retraction.
    let out_gm = filter_command(&repo, &cmd, &ctx_for(other, WorldRole::Gm), &WorldCapDefaults::default()).await;
    let Operation::Update { changes, .. } = &out_gm.ops[0] else { panic!() };
    assert!(!changes.iter().any(|c| c.path == "/system/name"));
}
```
(Reuse the exact repo/`ctx_for` helpers the neighboring `filter_command_*` tests use; do not invent
a new harness.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd src/server && cargo test --lib permission_tightening`
Expected: FAIL (no synthesized retraction is emitted).

- [ ] **Step 3: Implement the retroactive pass**

In `src/server/src/data/permission.rs`, add the boundary-aware helper:
```rust
/// Whether a change path writes into any document's envelope `permissions` (top-level or
/// embedded), i.e. a `permissions` path segment. Used to trigger retroactive redaction.
fn touches_permissions(path: &str) -> bool {
    path.split('/').any(|seg| seg == "permissions")
}
```
Rewrite the `Update` arm's `kept` computation so the non-GM branch appends retractions when the
command tightened permissions (`hidden` is computed once and reused):
```rust
                let kept: Vec<FieldChange> = if access.see_gm_only {
                    changes.clone()
                } else {
                    let mut hidden = Vec::new();
                    collect_hidden(&cur, &access, "", &mut hidden);
                    let mut kept: Vec<FieldChange> =
                        changes.iter().filter_map(|ch| redact_change(ch, &hidden)).collect();
                    // Retroactive redaction: a permission-tightening Update must retract any field
                    // now hidden from this recipient, so a previously-delivered value cannot linger.
                    // old:null avoids leaking the real value in the pre-image; new:null clears it.
                    // Idempotent — re-nulling an already-absent field is harmless. Per-recipient:
                    // the owner's OwnerOrGm fields are NOT in `hidden` (can_see), so stay intact.
                    if changes.iter().any(|c| touches_permissions(&c.path)) {
                        for ptr in hidden {
                            kept.push(FieldChange {
                                path: ptr,
                                old: serde_json::Value::Null,
                                new: serde_json::Value::Null,
                            });
                        }
                    }
                    kept
                };
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src/server && cargo test`
Expected: PASS (the new retroactive test + all existing redaction tests).

- [ ] **Step 5: Lint + commit**

```bash
cd src/server && cargo fmt --check && cargo clippy -- -D warnings
cd ../.. && git add src/server/src/data/permission.rs
git commit -m "feat(m10b): retroactive redaction on permission tightening"
```

---

## Task 3: Name display accessor + `setNameHidden` builder helper

**Files:**
- Modify: `src/client/core/src/actor.ts` (add `actorDisplayName`)
- Modify: `src/client/core/src/scene-docs.ts` (add `setNameHidden`)
- Modify: `src/client/core/src/index.ts` (exports)
- Test: `src/client/core/src/actor.test.ts`, `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Produces:
  ```ts
  function actorDisplayName(a: { name?: string; displayName?: string }, fallback?: string): string
  function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument
  ```

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/actor.test.ts`:
```ts
import { actorDisplayName } from "./actor";

describe("actorDisplayName", () => {
  it("prefers the real name, then displayName, then a generic fallback", () => {
    expect(actorDisplayName({ name: "Goblin Skirmisher", displayName: "Goblin" })).toBe("Goblin Skirmisher");
    expect(actorDisplayName({ displayName: "Goblin" })).toBe("Goblin");          // name stripped on egress
    expect(actorDisplayName({})).toBe("Unknown Creature");                        // fail-closed generic
    expect(actorDisplayName({}, "Mystery")).toBe("Mystery");
  });
});
```
Add to `src/client/core/src/scene-docs.test.ts`:
```ts
import { buildActorDoc, setNameHidden } from "./scene-docs";

describe("setNameHidden", () => {
  it("sets and clears the OwnerOrGm override on /system/name", () => {
    const d = buildActorDoc("w1", sys, "act1");          // `sys` from the existing actor test fixture
    setNameHidden(d, true);
    expect(d.permissions.property_overrides["/system/name"]).toBe("owner_or_gm");
    setNameHidden(d, false);
    expect(d.permissions.property_overrides["/system/name"]).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `pnpm --filter @shadowcat/core test actor scene-docs`
Expected: FAIL (`actorDisplayName`/`setNameHidden` not exported).

- [ ] **Step 3: Implement**

Append to `src/client/core/src/actor.ts`:
```ts
/** The name to show for an actor: the real name when present, else the non-secret
 * displayName, else a generic fallback. For unauthorized recipients the server has stripped
 * the real `name` (OwnerOrGm §7), so it is absent here — fail-closed: a missing name yields
 * the generic label, never a leak. The single display chokepoint every surface reads. */
export function actorDisplayName(a: { name?: string; displayName?: string }, fallback = "Unknown Creature"): string {
  return a.name || a.displayName || fallback;
}
```
Append to `src/client/core/src/scene-docs.ts`:
```ts
/** Set/clear the name-privacy override on an actor doc's permissions: hiding declares
 * `/system/name` as `OwnerOrGm` (the server redacts it from non-owner players on egress);
 * clearing removes the declaration. Mutates in place + returns `doc`. Leak-free only when set
 * BEFORE the name is delivered (§7's up-front model) — hiding an already-delivered name does
 * not retract it from clients that already hold it. */
export function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/system/name"] = "owner_or_gm";
  else delete overrides["/system/name"];
  doc.permissions = { ...doc.permissions, property_overrides: overrides };
  return doc;
}
```

- [ ] **Step 4: Export + verify pass**

`src/client/core/src/index.ts`:
```ts
export { buildSceneDoc, buildTokenDoc, buildSceneEntityDoc, buildActorDoc, buildTokenFromActor, setNameHidden } from "./scene-docs";
export { resolveTokenActor, actorDisplayName } from "./actor";
```
Run: `pnpm --filter @shadowcat/core test actor scene-docs && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/actor.ts src/client/core/src/scene-docs.ts src/client/core/src/actor.test.ts src/client/core/src/scene-docs.test.ts src/client/core/src/index.ts
git commit -m "feat(m10b): actorDisplayName accessor + setNameHidden helper"
```

---

## Task 4: Hide-name control in the Actors panel

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts` (i18n keys)

**Interfaces:**
- Consumes: `setNameHidden`, `actorDisplayName` (Task 3); `ctx.role`, `ctx.dispatchIntent`.
- Produces: a create-time "hide name" checkbox (leak-free) + a per-actor GM toggle that
  Updates `/permissions/property_overrides`; the list shows `actorDisplayName`.

- [ ] **Step 1: Add the i18n keys**

`src/client/ui-kit/src/locales/en.ts` (after the `actors.*` block):
```ts
  "actors.hideName": "Hide name from players",
  "actors.nameHidden": "Name hidden",
  "actors.nameShown": "Name shown",
```

- [ ] **Step 2: Wire the create-time checkbox + display accessor + toggle**

In `src/modules/actors/src/ActorsPanel.svelte` `<script>`: import the helpers and add state:
```ts
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, type ActorSystem, type WireDocument } from "@shadowcat/core";
  // …
  let hideName = $state(false);
```
In `create()`, build the doc, apply the override, then dispatch:
```ts
    const doc = buildActorDoc(ctx.world, system);
    if (hideName) setNameHidden(doc, true);
    ctx.dispatchIntent([{ op: "create", doc }]);
    name = "";
    displayName = "";
    assetId = null;
    hideName = false;
```
Add a per-actor GM toggle (Update the permissions map — replace wholesale so un-hiding can
remove the key):
```ts
  function toggleHidden(a: WireDocument): void {
    const cur = a.permissions.property_overrides;
    const next = { ...cur };
    const hidden = next["/system/name"] === "owner_or_gm";
    if (hidden) delete next["/system/name"];
    else next["/system/name"] = "owner_or_gm";
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/permissions/property_overrides", old: cur, new: next }] }]);
  }
  const isHidden = (a: WireDocument): boolean => a.permissions.property_overrides["/system/name"] === "owner_or_gm";
```
In the list item, show the safe display name and (GM only) the toggle:
```svelte
    {#each actorDocs as a (a.id)}
      <li>
        <button type="button" class:selected={ctx.actorSelection.selectedId === a.id} onclick={() => ctx.actorSelection.select(a.id)}>
          {actorDisplayName(a.system as { name?: string; displayName?: string })}
        </button>
        {#if ctx.role === "gm"}
          <button type="button" class="hide-toggle" onclick={() => toggleHidden(a)}>
            {isHidden(a) ? t("actors.nameShown") : t("actors.hideName")}
          </button>
        {/if}
      </li>
    {/each}
```
In the create `<form>`, add the checkbox:
```svelte
    <label><input type="checkbox" bind:checked={hideName} /> {t("actors.hideName")}</label>
```

- [ ] **Step 3: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-actors test && pnpm -r typecheck`
Expected: PASS.
```bash
git add src/modules/actors/src/ActorsPanel.svelte src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10b): actors panel hide-name control + safe display name"
```

---

## Task 5: Faction registry types + builder

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Modify: `src/client/core/src/index.ts`
- Test: `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Produces:
  ```ts
  type FactionStance = "friendly" | "neutral" | "hostile";
  interface Faction { name: string; color: string; stance: FactionStance }
  interface FactionRegistrySystem { factions: Record<string, Faction> }
  function buildFactionRegistryDoc(worldId: string, factions: Record<string, Faction>, id?: string): WireDocument
  ```

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/scene-docs.test.ts`:
```ts
import { buildFactionRegistryDoc, type Faction } from "./scene-docs";

describe("buildFactionRegistryDoc", () => {
  it("builds a world-scoped, parentless faction-registry with an id→faction map", () => {
    const factions: Record<string, Faction> = { hostile: { name: "Hostile", color: "#f85149", stance: "hostile" } };
    const d = buildFactionRegistryDoc("w1", factions, "reg1");
    expect(d.doc_type).toBe("faction-registry");
    expect(d.parent_id).toBeNull();
    expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
    expect((d.system as { factions: unknown }).factions).toEqual(factions);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: FAIL ("buildFactionRegistryDoc is not a function").

- [ ] **Step 3: Implement**

Add to `src/client/core/src/scene-docs.ts`:
```ts
/** A faction's display + stance. `color` is "#rrggbb" (the token border color); `stance` is
 * reserved for later combat/targeting/vision (present now to avoid a migration §6). */
export type FactionStance = "friendly" | "neutral" | "hostile";
export interface Faction {
  name: string;
  color: string;
  stance: FactionStance;
}

/** The world's faction registry: a singleton config document (doc_type "faction-registry").
 * `factions` is keyed by faction id — an actor's `faction` field references a key. A MAP, not
 * an array, so adding a faction is a single-key Update (`set_pointer` cannot grow arrays). */
export interface FactionRegistrySystem {
  factions: Record<string, Faction>;
}

export function buildFactionRegistryDoc(worldId: string, factions: Record<string, Faction>, id?: string): WireDocument {
  return envelope(worldId, "faction-registry", null, { factions } satisfies FactionRegistrySystem, id);
}
```

- [ ] **Step 4: Export + verify pass**

`src/client/core/src/index.ts`: add `buildFactionRegistryDoc` to the scene-docs value export and
`Faction, FactionStance, FactionRegistrySystem` to the scene-docs type export.
Run: `pnpm --filter @shadowcat/core test scene-docs && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts src/client/core/src/index.ts
git commit -m "feat(m10b): faction registry types + buildFactionRegistryDoc"
```

---

## Task 6: `module-factions` — editor + idempotent GM seed

**Files:**
- Create: `src/modules/factions/package.json`, `tsconfig.json`, any vitest/svelte config
  (copy from `src/modules/actors`)
- Create: `src/modules/factions/src/index.ts`, `src/modules/factions/src/FactionsPanel.svelte`
- Test: `src/modules/factions/src/index.test.ts`
- Modify: `src/client/shell/src/App.svelte` (import + modules array), `src/client/shell/package.json`
- Modify: `src/client/ui-kit/src/locales/en.ts` (i18n keys)

**Interfaces:**
- Consumes: `buildFactionRegistryDoc`, `Faction`, `FactionRegistrySystem` (Task 5); `getAppContext`.
- Produces: `export const factions: Module` contributing `FactionsPanel` into
  `shadowcat.surface:sidebar` (order 3); seeds three defaults once per world (GM, when absent).

- [ ] **Step 1: Write the failing test**

Create `src/modules/factions/src/index.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { factions } from "./index";

describe("factions module", () => {
  it("contributes a sidebar panel and requires the sidebar surface", () => {
    expect(factions.manifest.id).toBe("factions");
    expect(factions.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    factions.register({ contributions } as any);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/module-factions test`
Expected: FAIL (package/module missing).

- [ ] **Step 3: Create the package + module entry**

`src/modules/factions/package.json` (mirror `src/modules/actors/package.json`, including
`@shadowcat/types`); copy `tsconfig.json` + any vitest/svelte config from `src/modules/actors`.
Run `pnpm install` to link the workspace package.
`src/modules/factions/src/index.ts`:
```ts
import type { Module } from "@shadowcat/core";
import FactionsPanel from "./FactionsPanel.svelte";

/** World faction registry: seeds three defaults (GM, idempotent) and provides the GM editor.
 * Replaceable — a game-system module can supply its own seed/editor. Requires core-ui's sidebar. */
export const factions: Module = {
  manifest: {
    id: "factions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "factions:sidebar", contract: "shadowcat.surface:sidebar", order: 3, component: FactionsPanel });
  },
};
```

- [ ] **Step 4: Implement the panel (editor + seed)**

`src/modules/factions/src/FactionsPanel.svelte`:
```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildFactionRegistryDoc, type Faction, type FactionRegistrySystem, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("faction-registry")[0];
  });
  const factionEntries = $derived.by((): [string, Faction][] => {
    const sys = registry?.system as FactionRegistrySystem | undefined;
    return Object.entries(sys?.factions ?? {});
  });

  // Idempotent GM seed: create the registry with three defaults once, only when absent. The
  // optimistic dispatch adds it to the store immediately, so a second reactive run sees it.
  const SEED: Record<string, Faction> = {
    friendly: { name: "Friendly", color: "#3fb950", stance: "friendly" },
    neutral: { name: "Neutral", color: "#9e9e9e", stance: "neutral" },
    hostile: { name: "Hostile", color: "#f85149", stance: "hostile" },
  };
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    if (ctx.documents.query("faction-registry").length > 0) { seeded = true; return; }
    seeded = true;
    ctx.dispatchIntent([{ op: "create", doc: buildFactionRegistryDoc(ctx.world, SEED) }]);
  });

  function update(id: string, patch: Partial<Faction>): void {
    if (!registry) return;
    for (const [k, v] of Object.entries(patch)) {
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/factions/${id}/${k}`, old: null, new: v }] }]);
    }
  }
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/factions/${id}`, old: null, new: { name: "New faction", color: "#9e9e9e", stance: "neutral" } satisfies Faction }] }]);
  }
  function remove(id: string): void {
    const sys = registry?.system as FactionRegistrySystem | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.factions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/system/factions", old: sys.factions, new: next }] }]);
  }
</script>

<section class="factions">
  <h3>{t("factions.title")}</h3>
  <ul class="list">
    {#each factionEntries as [id, f] (id)}
      <li>
        <span class="swatch" style="background:{f.color}"></span>
        {#if ctx.role === "gm"}
          <input aria-label={t("factions.name")} value={f.name} onchange={(e) => update(id, { name: e.currentTarget.value })} />
          <input type="color" aria-label={t("factions.color")} value={f.color} onchange={(e) => update(id, { color: e.currentTarget.value })} />
          <select aria-label={t("factions.stance")} value={f.stance} onchange={(e) => update(id, { stance: e.currentTarget.value as Faction["stance"] })}>
            <option value="friendly">{t("factions.friendly")}</option>
            <option value="neutral">{t("factions.neutral")}</option>
            <option value="hostile">{t("factions.hostile")}</option>
          </select>
          <button type="button" onclick={() => remove(id)}>{t("factions.remove")}</button>
        {:else}
          <span>{f.name}</span>
        {/if}
      </li>
    {/each}
  </ul>
  {#if ctx.role === "gm"}
    <button type="button" onclick={add}>{t("factions.add")}</button>
  {/if}
</section>

<style lang="scss">
  .factions { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .list li { display: flex; align-items: center; gap: var(--space-1); }
  .swatch { width: 16px; height: 16px; border-radius: var(--radius-1); border: 1px solid var(--border); flex: 0 0 auto; }
  input, select, button { min-height: 32px; }
</style>
```

- [ ] **Step 5: Add the i18n keys**

`src/client/ui-kit/src/locales/en.ts`:
```ts
  "factions.title": "Factions",
  "factions.name": "Name",
  "factions.color": "Color",
  "factions.stance": "Stance",
  "factions.friendly": "Friendly",
  "factions.neutral": "Neutral",
  "factions.hostile": "Hostile",
  "factions.add": "Add faction",
  "factions.remove": "Remove",
  "factions.selectTokens": "Select tokens",
```

- [ ] **Step 6: Register in the shell**

`src/client/shell/src/App.svelte`: add `import { factions } from "@shadowcat/module-factions";`
and append `factions` to the modules array (line 82):
```ts
    modules: [coreUi, topBar, statusBar, stage, settings, assets, actors, factions, sceneTools]
```
Add `@shadowcat/module-factions` to `src/client/shell/package.json` dependencies; run
`pnpm install`.

- [ ] **Step 7: Verify + commit**

Run: `pnpm --filter @shadowcat/module-factions test && pnpm -r typecheck`
Expected: PASS.
```bash
git add -A
git commit -m "feat(m10b): module-factions (GM editor + idempotent seed) + shell registration"
```

---

## Task 7: Faction assignment in the Actors panel

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`

**Interfaces:**
- Consumes: `FactionRegistrySystem`, `Faction` (Task 5); the seeded registry (Task 6).
- Produces: a create-time faction `<select>` (sets `actor.system.faction`) + a per-actor faction
  picker (Updates `/system/faction`).

- [ ] **Step 1: Add the i18n key**

`src/client/ui-kit/src/locales/en.ts`: `"actors.faction": "Faction",`

- [ ] **Step 2: Wire the faction select**

In `ActorsPanel.svelte` `<script>`, derive faction options from the registry and add create state:
```ts
  import { type FactionRegistrySystem, type Faction } from "@shadowcat/core";
  let faction = $state<string | null>(null);
  const factionOptions = $derived.by((): [string, Faction][] => {
    subscribe();
    const reg = ctx.documents.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
    return Object.entries(reg?.factions ?? {});
  });
```
In the `ActorSystem` literal in `create()`, set `faction` (currently hard-coded `null`):
```ts
      faction,
```
Reset after create: `faction = null;`. Add the create-form select:
```svelte
    <label>{t("actors.faction")}
      <select bind:value={faction}>
        <option value={null}>—</option>
        {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
      </select>
    </label>
```
Add a per-actor faction picker in the list item:
```svelte
        {#if ctx.role === "gm"}
          <select
            aria-label={t("actors.faction")}
            value={(a.system as { faction?: string | null }).faction ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/faction", old: (a.system as { faction?: string | null }).faction ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">—</option>
            {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
          </select>
        {/if}
```

- [ ] **Step 3: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-actors test && pnpm -r typecheck`
Expected: PASS.
```bash
git add src/modules/actors/src/ActorsPanel.svelte src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10b): assign factions to actors in the actors panel"
```

---

## Task 8: Faction-colored token border

**Files:**
- Modify: `src/client/render/src/types.ts` (`TokenNodeSpec`)
- Modify: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/pixi-backend.ts`
- Test: `src/client/render/src/token-view.test.ts`

**Interfaces:**
- Consumes: `resolveTokenActor`, `FactionRegistrySystem` (`@shadowcat/core`); `parseColor`
  (`./geometry`).
- Produces: `TokenNodeSpec` gains `borderColor: number | null`; TokenView resolves it from the
  token's faction via the registry; the Pixi backend strokes a per-token border.

- [ ] **Step 1: Write the failing test**

Add to `src/client/render/src/token-view.test.ts` (mirror the existing linked-token harness;
seed the store with a `faction-registry` doc and an actor with `faction:"f1"`):
```ts
it("resolves the faction border color from the registry", () => {
  // store: faction-registry { factions: { f1: { name:"F1", color:"#ff0000", stance:"hostile" } } }
  //        actor "act1" with system.faction = "f1", visual asset "a1"
  //        token "tok1" linked to act1
  view.reconcile();
  expect(backend.tokens.get("tok1")?.borderColor).toBe(0xff0000);
});
it("has a null border when the token has no faction", () => {
  // store: actor with faction:null + its linked token
  view.reconcile();
  expect(backend.tokens.get("tok2")?.borderColor).toBeNull();
});
```
(Use the test file's existing `MockBackend`/store helpers; the mock records the last
`TokenNodeSpec` per id in `backend.tokens`.)

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test token-view`
Expected: FAIL (`borderColor` absent from the spec).

- [ ] **Step 3: Extend `TokenNodeSpec`**

`src/client/render/src/types.ts`:
```ts
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  url: string;
  /** Faction border color (0xRRGGBB), or null for no border. */
  borderColor: number | null;
}
```

- [ ] **Step 4: Resolve the faction color in TokenView**

`src/client/render/src/token-view.ts`:
```ts
import { resolveTokenActor, type ReadableDocuments, type AssetResolver, type WireDocument, type FactionRegistrySystem } from "@shadowcat/core";
import { parseColor } from "./geometry";
// …
  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.system as TokenSystem | undefined;
    if (!s) return null;
    const eff = resolveTokenActor(doc, this.store);
    const visual = eff?.visual ?? s.visual;
    if (visual?.kind !== "image") return null;
    let borderColor: number | null = null;
    if (eff?.faction) {
      const reg = this.store.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
      const hex = reg?.factions?.[eff.faction]?.color;
      if (hex) borderColor = parseColor(hex);
    }
    return { x: s.x, y: s.y, w: s.w, h: s.h, rotation: s.rotation ?? 0, url: this.assets.url(visual.asset), borderColor };
  }
```

- [ ] **Step 5: Stroke the border in the Pixi backend**

`src/client/render/src/pixi-backend.ts` — add a border-Graphics map and draw/clear it in
`setToken`, destroying it in `removeToken`:
```ts
  private readonly tokenBorders = new Map<string, Graphics>();
  // …in setToken(), after setting sprite transform:
    let border = this.tokenBorders.get(id);
    if (spec.borderColor === null) {
      if (border) { border.destroy(); this.tokenBorders.delete(id); }
    } else {
      if (!border) {
        border = new Graphics();
        this.tokenBorders.set(id, border);
        this.layers.get("tokens")?.addChild(border);
      }
      const hw = spec.w / 2, hh = spec.h / 2;
      border.clear();
      border.rect(-hw, -hh, spec.w, spec.h).stroke({ width: 3, color: spec.borderColor });
      border.position.set(spec.x, spec.y);
      border.angle = spec.rotation; // degrees, like the sprite
    }
  // …in removeToken(), alongside the sprite cleanup:
    const b = this.tokenBorders.get(id);
    if (b) { b.destroy(); this.tokenBorders.delete(id); }
```

- [ ] **Step 6: Run tests to verify pass**

Run: `pnpm --filter @shadowcat/render test token-view`
Expected: PASS (new border cases + existing visual cases — the existing cases now also set
`borderColor: null`, so update any exact-object assertions in the file accordingly).

- [ ] **Step 7: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/token-view.ts src/client/render/src/pixi-backend.ts src/client/render/src/token-view.test.ts
git commit -m "feat(m10b): faction-colored token border"
```

---

## Task 9: `TokenSelection` holder + AppContext/ToolContext wiring

**Files:**
- Create: `src/client/ui-kit/src/tokenSelection.svelte.ts`
- Modify: `src/client/ui-kit/src/index.ts`, `src/client/ui-kit/src/appContext.ts`
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`, `src/client/shell/src/lib/Table.svelte`
- Modify: fixtures: `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`,
  `src/client/ui-kit/src/__fixtures__/appContextTest.ts`,
  `src/modules/assets/src/__fixtures__/AssetsHarness.svelte`
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`ToolContext`),
  `src/modules/scene-tools/src/ToolRail.svelte` (pass-through)
- Test: `src/client/ui-kit/src/tokenSelection.test.ts`

**Interfaces:**
- Produces:
  ```ts
  class TokenSelection {
    readonly ids: ReadonlySet<string>;
    has(id: string): boolean;
    set(ids: Iterable<string>): void;
    toggle(id: string): void;
    clear(): void;
  }
  // AppContext gains:  tokenSelection: TokenSelection   (required)
  // ToolContext gains: tokenSelection?: TokenSelection  (optional — existing tool tests unaffected)
  ```

- [ ] **Step 1: Write the failing test**

Create `src/client/ui-kit/src/tokenSelection.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { TokenSelection } from "./tokenSelection.svelte";

describe("TokenSelection", () => {
  it("sets, toggles, and clears the selected token ids", () => {
    const sel = new TokenSelection();
    expect(sel.has("a")).toBe(false);
    sel.set(["a", "b"]);
    expect([...sel.ids].sort()).toEqual(["a", "b"]);
    sel.toggle("b");
    expect(sel.has("b")).toBe(false);
    sel.toggle("c");
    expect(sel.has("c")).toBe(true);
    sel.clear();
    expect(sel.ids.size).toBe(0);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test tokenSelection`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement the holder**

Create `src/client/ui-kit/src/tokenSelection.svelte.ts`:
```ts
import { SvelteSet } from "svelte/reactivity";

/** The set of selected token ids (group-select). A stable instance held by WorldSession and
 * shared via AppContext (the factions panel sets it; the select tool reads + moves it). Backed
 * by a SvelteSet so panel reads are reactive; mutated in place — never reassigned. */
export class TokenSelection {
  #ids = new SvelteSet<string>();
  get ids(): ReadonlySet<string> { return this.#ids; }
  has(id: string): boolean { return this.#ids.has(id); }
  set(ids: Iterable<string>): void {
    this.#ids.clear();
    for (const id of ids) this.#ids.add(id);
  }
  toggle(id: string): void {
    if (!this.#ids.delete(id)) this.#ids.add(id);
  }
  clear(): void { this.#ids.clear(); }
}
```

- [ ] **Step 4: Wire types + construction**

`src/client/ui-kit/src/index.ts`: `export { TokenSelection } from "./tokenSelection.svelte";`
`src/client/ui-kit/src/appContext.ts` — import + add to `AppContext` (near `actorSelection`):
```ts
import type { TokenSelection } from "./tokenSelection.svelte";
// …inside AppContext:
  /** Selected token ids for group-select; set by the factions panel, read by the select tool. */
  tokenSelection: TokenSelection;
```
`src/client/shell/src/lib/worldSession.svelte.ts` (near `actorSelection`, line ~48):
```ts
import { SceneInteractionBridge, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
// …
  readonly tokenSelection = new TokenSelection();
```
`src/client/shell/src/lib/Table.svelte` — add `tokenSelection: session.tokenSelection` to the
`setAppContext({ … })` literal. Add `tokenSelection: new TokenSelection()` (import from
`@shadowcat/ui-kit`) to the three fixtures listed in **Files**.
`src/modules/scene-tools/src/controller.svelte.ts` — add to `ToolContext`:
```ts
import type { SceneInteraction, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
// …inside ToolContext:
  /** Selected token ids (group-select); the select tool reads + moves the whole set. */
  tokenSelection?: TokenSelection;
```
`src/modules/scene-tools/src/ToolRail.svelte` — add `tokenSelection: ctx.tokenSelection` to the
`new ToolController({ … })` literal (the AppContext slice it builds).

- [ ] **Step 5: Typecheck + tests pass**

Run: `pnpm -r typecheck && pnpm --filter @shadowcat/ui-kit test tokenSelection`
Expected: PASS (all AppContext literals satisfy the interface).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(m10b): TokenSelection seam on AppContext + ToolContext"
```

---

## Task 10: Group-select — multi-drag + selection overlay + faction select

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makeSelectMoveTool`)
- Modify: `src/modules/factions/src/FactionsPanel.svelte` (per-faction "Select tokens")
- Test: `src/modules/scene-tools/src/select-move-tool.test.ts` (create if absent; else extend
  the existing select-tool test file)

**Interfaces:**
- Consumes: `TokenSelection` via `ToolContext` (Task 9); `resolveTokenActor` (`@shadowcat/core`);
  `topTokenAt` (`./hit-test`).
- Produces: clicking a token selects just it (Shift toggles); dragging a selected token moves
  the whole selection by the snapped delta; selected tokens get an overlay ring; the factions
  panel can select all of a faction's tokens.

- [ ] **Step 1: Write the failing test**

Add to the select-tool test (provide `ctxWith` that includes a `TokenSelection` and a store with
two tokens at known centers; inject `now`):
```ts
it("moves all selected tokens together by the snapped delta", () => {
  const { ctx, sent } = ctxWith(storeWithTwoTokens()); // tok1 @ (100,100), tok2 @ (300,100)
  ctx.tokenSelection!.set(["tok1", "tok2"]);
  const tool = makeSelectMoveTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, { shiftKey: false } as PointerEvent); // grab tok1
  tool.onPointerMove({ x: 200, y: 100 }, {} as PointerEvent);                  // +100 in x
  tool.onPointerUp({ x: 200, y: 100 }, {} as PointerEvent);
  // both tokens moved +100 in x (tok1 -> 200, tok2 -> 400)
  const moves = sent.flat().filter((o: any) => o.op === "update");
  const xByDoc = new Map(moves.map((m: any) => [m.doc_id, m.changes.find((c: any) => c.path === "/system/x").new]));
  expect(xByDoc.get("tok1")).toBe(200);
  expect(xByDoc.get("tok2")).toBe(400);
});

it("clicking an unselected token replaces the selection with just it", () => {
  const { ctx } = ctxWith(storeWithTwoTokens());
  ctx.tokenSelection!.set(["tok2"]);
  const tool = makeSelectMoveTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, { shiftKey: false } as PointerEvent); // grab tok1
  expect([...ctx.tokenSelection!.ids]).toEqual(["tok1"]);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-tools test select`
Expected: FAIL (single-token drag only; no group move).

- [ ] **Step 3: Rewrite `makeSelectMoveTool` for group move + overlay**

`src/modules/scene-tools/src/controller.svelte.ts`:
```ts
/** Pick a token on pointerdown and drag it (and the whole selection). Clicking an unselected
 * token replaces the selection with just it; Shift toggles it in/out. Dragging moves every
 * selected token by the same snapped delta, preserving relative offsets. Clicking empty space
 * clears the selection (and yields the gesture to the camera). A faction-colored ring overlay
 * marks the selection. */
export function makeSelectMoveTool(ctx: ToolContext): SceneTool {
  const now = ctx.now ?? ((): number => Date.now());
  const sel = ctx.tokenSelection;
  let draggingId: string | null = null;
  let grabOrigin: Point = { x: 0, y: 0 };
  let origins = new Map<string, Point>(); // selected id -> original center
  let moved = false;
  let lastSentAt = -Infinity;

  const centerOf = (id: string): Point => {
    const s = ctx.documents.get(id)?.system as { x?: number; y?: number; w?: number; h?: number } | undefined;
    return { x: s?.x ?? 0, y: s?.y ?? 0 };
  };
  const sizeOf = (id: string): { w: number; h: number } => {
    const s = ctx.documents.get(id)?.system as { w?: number; h?: number } | undefined;
    return { w: s?.w ?? 100, h: s?.h ?? 100 };
  };

  /** A closed rect ring per selected token, into the tool overlay (cleared on empty). */
  const drawSelection = (): void => {
    if (!sel) return;
    const rings = [...sel.ids].map((id) => {
      const c = centerOf(id);
      const { w, h } = sizeOf(id);
      const hw = w / 2, hh = h / 2;
      return { points: [c.x - hw, c.y - hh, c.x + hw, c.y - hh, c.x + hw, c.y + hh, c.x - hw, c.y + hh], closed: true, stroke: { color: 0xffd400, width: 2 }, fill: null };
    });
    if (rings.length === 0) ctx.scene.clearOverlay();
    else ctx.scene.previewOverlay(rings);
  };

  const sendMoves = (delta: Point): void => {
    const ops: WireOperation[] = [];
    for (const [id, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      const sys = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
      ops.push({ op: "update", doc_id: id, changes: [
        { path: "/system/x", old: sys?.x ?? null, new: target.x },
        { path: "/system/y", old: sys?.y ?? null, new: target.y },
      ] });
    }
    if (ops.length > 0) ctx.dispatchIntent(ops);
  };

  return {
    onPointerDown(p: Point, ev: PointerEvent): boolean {
      const id = topTokenAt(ctx.documents.query("token"), p);
      if (!id) { sel?.clear(); ctx.scene.clearOverlay(); return false; }
      if (sel) {
        if (ev.shiftKey) sel.toggle(id);
        else if (!sel.has(id)) sel.set([id]);
      }
      draggingId = id;
      grabOrigin = { x: p.x, y: p.y };
      origins = new Map([...(sel?.ids ?? [id])].map((sid) => [sid, centerOf(sid)]));
      if (!origins.has(id)) origins.set(id, centerOf(id));
      moved = false;
      lastSentAt = -Infinity;
      ctx.scene.setDraggingToken(id);
      drawSelection();
      return true;
    },
    onPointerMove(p: Point): void {
      if (!draggingId) return;
      moved = true;
      const delta = { x: p.x - grabOrigin.x, y: p.y - grabOrigin.y };
      const t = now();
      if (t - lastSentAt >= DRAG_THROTTLE_MS) { sendMoves(delta); lastSentAt = t; }
      drawSelection();
    },
    onPointerUp(p: Point): void {
      if (!draggingId) return;
      if (moved) sendMoves({ x: p.x - grabOrigin.x, y: p.y - grabOrigin.y });
      ctx.scene.setDraggingToken(null);
      draggingId = null;
      moved = false;
      drawSelection();
    },
  };
}
```
(Add `resolveTokenActor` to the `@shadowcat/core` import only where Step 4 needs it; the tool
itself needs no new core import.)

- [ ] **Step 4: Faction "Select tokens" button**

In `src/modules/factions/src/FactionsPanel.svelte`, import the resolver and add a per-faction
GM button that selects every token of that faction on the active scene:
```ts
  import { resolveTokenActor } from "@shadowcat/core";
  function selectTokens(factionId: string): void {
    const ids = ctx.documents.query("token").filter((tok) => resolveTokenActor(tok, ctx.documents)?.faction === factionId).map((tok) => tok.id);
    ctx.tokenSelection.set(ids);
  }
```
In the GM branch of each faction row:
```svelte
          <button type="button" onclick={() => selectTokens(id)}>{t("factions.selectTokens")}</button>
```

- [ ] **Step 5: Run tests to verify pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test select && pnpm --filter @shadowcat/module-factions test && pnpm -r typecheck`
Expected: PASS (group move + selection-replace + the existing single-drag behavior, which is the
N=1 case of the new logic).

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/select-move-tool.test.ts src/modules/factions/src/FactionsPanel.svelte
git commit -m "feat(m10b): faction group-select (multi-drag + overlay + select-by-faction)"
```

---

## Task 11: Full-suite verification

- [ ] **Step 1: Run the whole client suite + typecheck + lint**

Run: `pnpm -r test && pnpm -r typecheck && pnpm lint`
Expected: PASS.

- [ ] **Step 2: Run the server suite + format/lint + ts-rs sync**

Run: `cd src/server && cargo test && cargo fmt --check && cargo clippy -- -D warnings`
Expected: PASS; `git status` shows no uncommitted regenerated `src/types/generated/*`.

- [ ] **Step 3: Manual smoke (optional, GM + player flow)**

Build the client (`pnpm --filter @shadowcat/ui build`), run the server binary. As GM: the
Factions panel seeds three defaults; edit a color; create an actor, assign it a faction, place a
token — it shows the faction-colored border; "Select tokens" selects the faction's tokens and a
drag moves them together. Create an actor with "hide name" checked; as a player in the same world,
the actor/token surfaces show the `displayName`, never the real name. With the player already
viewing an actor, the GM toggles "hide name" on — the player's view falls back to `displayName`
(retroactive redaction, Task 2b), confirming no stale name lingers.

---

## Self-review (completed)

- **Spec coverage:**
  - §6 Factions — registry config-doc (Task 5), seed module + GM editor (Task 6), faction
    reference on actors (Task 7), token border color (Task 8), group-select (Tasks 9–10),
    `stance` reserved (carried in `Faction`, unused). ✓
  - §7 Name privacy — `OwnerOrGm` tier (Task 1), owner-aware redaction across all egress paths
    incl. embedded + search (Task 2), retroactive redaction on permission tightening (Task 2b),
    `displayName` accessor chokepoint (Task 3), GM hide control (Task 4), fail-closed (the
    missing-name fallback). ✓
  - §13.10 World registries are config-documents — faction-registry singleton (Task 5/6). ✓
- **Type consistency:** `Faction`/`FactionRegistrySystem` defined in Task 5, consumed unchanged
  in 6/7/8/10; `TokenSelection` defined in Task 9, consumed in 10; `Access::is_owner`/`can_see`
  defined in Task 2, the single redaction predicate. `TokenNodeSpec.borderColor` added in Task 8
  — existing token-view assertions updated in the same task.
- **Placeholder scan:** none — each code step carries real code or an exact command.
- **Decisions recorded:** name-privacy is up-front AND retroactively enforced on permission
  tightening (Task 2b, user-chosen full scope) — no residual leak; the faction registry is a map
  (Update-safe); the GM-seed multi-client simultaneous-first-entry race is the only residual
  idempotency gap (single-GM is the norm), noted in Task 6.
