# M13-0 · Three-Band Document Shape — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure every document from `envelope + system` to `envelope(+name) + engine? + system` per the approved spec `docs/superpowers/specs/2026-07-15-m13-0-document-shape-design.md` (S1–S7) — typed, ts-rs-generated `engine` band with strict ingress; `system` handed to the game system; hard cutover, zero migration.

**Architecture:** Server-first. Tasks 1–7 restructure the Rust side and keep `cargo test` green per task (server tests re-root as they go). Tasks 8–9 re-root the client against the newly generated types and keep `pnpm -r test` + `typecheck` green per task. Task 10 is the cross-boundary gate: full e2e + stale-ref sweep + ingress-rejection battery. The wire protocol is BROKEN between Task 1 and Task 8 by design (hard cutover); e2e is only expected green at Task 10.

**Tech Stack:** Rust (serde, ts-rs), TypeScript (Zod at the envelope only), Vitest, existing node↔Rust e2e harness.

## Global Constraints

- **No new dependencies** (server or client). No DB schema change (documents persist as one JSON column — envelope fields are serde-only).
- **Zero migration code** (spec §1; pre-v1, no shipped worlds).
- **Semantics-preserving re-roots**: every relocated read/write keeps today's behavior exactly — fail-closed defaults (bounds → `DEFAULT_SCENE_BOUNDS_UNITS`, grid size → 100 and > 0, clamps) survive as serde defaults + read-side backstops. Ingress validation is **shape/type only** in this checkpoint; range clamps stay read-side (spec: stricter never looser — and never looser means no existing read-clamp is removed).
- **`dist/` before cargo**: every server build/test in CI and locally needs the client built first (embed ordering). Run cargo via subshell or `--manifest-path`; never `cd` the session (cwd-drift hazard).
- **Reserved keys**: `modules` must remain rejected at the document root (S4) — `deny_unknown_fields` on `Document` provides this; no task may remove that attribute.
- **Tests yield to correct code** only when objectively wrong; here the existing suites are the oracle for preserved semantics — re-root paths/fixtures, never weaken assertions.
- Commit per task once its scope's tests are green. Branch: `m13-0-document-shape` in a git worktree off local `main`.

---

### Task 1: Envelope gains `name` + `engine` (server)

**Files:**
- Modify: `src/server/src/data/document.rs` (Document struct, ~line 204)
- Modify: every server test fixture constructing a `Document` literal (compile-driven sweep; e.g. `validation.rs` tests, `permission.rs` tests, `chat/mod.rs` tests)
- Generated: `src/types/generated/Document.ts` (regenerated, never hand-edited)

**Interfaces:**
- Produces: `Document.name: Option<String>`, `Document.engine: Option<serde_json::Value>` — every later task reads/writes these.

- [ ] **Step 1: Write the failing test** (in `document.rs` tests):

```rust
#[test]
fn document_carries_name_and_engine_and_rejects_modules_key() {
    let json = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "token", "schema_version": 1,
        "name": "Goblin", "engine": {"x": 1.0},
        "system": {}, "created_at": 0, "updated_at": 0
    });
    let doc: Document = serde_json::from_value(json).unwrap();
    assert_eq!(doc.name.as_deref(), Some("Goblin"));
    assert!(doc.engine.is_some());

    // absent name/engine default to None (serde default)
    let bare = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "note", "schema_version": 1, "system": {}, "created_at": 0, "updated_at": 0
    });
    let doc: Document = serde_json::from_value(bare).unwrap();
    assert!(doc.name.is_none() && doc.engine.is_none());

    // S4 reservation: unknown root key `modules` is rejected
    let with_modules = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "note", "schema_version": 1, "system": {}, "modules": {},
        "created_at": 0, "updated_at": 0
    });
    assert!(serde_json::from_value::<Document>(with_modules).is_err());
}
```

- [ ] **Step 2: Run** (`cargo test -p shadowcat document_carries` from `src/server` in a subshell) — expect FAIL (unknown fields `name`/`engine`).
- [ ] **Step 3: Implement** — add to `Document` between `schema_version` and `source`:

```rust
    /// Universal display name (S2). Redacts to `null` under a `/name` override.
    #[serde(default)]
    pub name: Option<String>,
```

and between `parent_id` and `system`:

```rust
    /// Engine band (S1/S3): present iff `doc_type` is engine-defined; validated
    /// against the doc_type's typed struct at ingress (data/engine). Stored
    /// post-validation. `None` for community/system doc types.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub engine: Option<serde_json::Value>,
```

- [ ] **Step 4: Compile-driven fixture sweep** — `cargo test` will now fail to compile every `Document { … }` literal; add `name: None, engine: None,` to each (tests + any constructor in non-test code, e.g. server-side doc builders in `chat/mod.rs`). Run full `cargo test`: PASS (bindings regenerate as a side effect of the ts-rs export tests; verify `src/types/generated/Document.ts` now contains `name: string | null` and `engine: unknown` — NOTE: `#[ts(type = "unknown")]` overrides ts-rs's `Option` handling, so the generated key is REQUIRED and un-unioned, same as `system` today; Task 8 must not assume `engine` is omittable in the generated type — the Zod schema's `z.unknown()` inferring optional is the runtime-tolerant side).
- [ ] **Step 5: Commit** — `feat(m13-0): envelope name + engine band on Document (server)`

---

### Task 2: `data/engine/` — typed engine structs + registry

**Files:**
- Create: `src/server/src/data/engine/mod.rs` (registry + `validate_engine` + `engine_of` helper)
- Create: `src/server/src/data/engine/token.rs` (TokenEngine, TokenOverrides, TokenVisual, RenderVisual, AnimatedSource, VisionAssignment, ActorEngine)
- Create: `src/server/src/data/engine/scene.rs` (SceneEngine + overrides, WorldSettingsEngine + defaults, LightEngine, VisionModesEngine, LightGradationEngine, shared enums)
- Create: `src/server/src/data/engine/geometry.rs` (WallEngine, RegionEngine, DrawingEngine, TemplateEngine)
- Create: `src/server/src/data/engine/registries.rs` (ChannelRegistryEngine, FactionRegistryEngine, ConditionRegistryEngine, ChatSettingsEngine, DiceSettingsEngine)
- Modify: `src/server/src/data/mod.rs` (add `pub mod engine;`), `DataError` (add `BadEngine(String)` variant if no fitting variant exists)
- Generated: `src/types/generated/engine/*.ts`

**Interfaces:**
- Produces (consumed by Tasks 3–9):

```rust
/// Ok(()) iff `engine` is valid for `doc_type`: engine doc types must carry a
/// body that deserializes into their struct; all other doc types must carry None.
pub fn validate_engine(doc_type: &str, engine: Option<&serde_json::Value>) -> Result<(), DataError>;

/// Fail-closed typed read: the stored engine (validated at ingress) or T::default().
pub fn engine_of<T: serde::de::DeserializeOwned + Default>(doc: &Document) -> T;

/// Some(typed check) for engine doc types, None otherwise.
pub fn is_engine_doc_type(doc_type: &str) -> bool;
```

- [ ] **Step 1: Write the structs.** Field shapes are the CURRENT client interfaces verbatim (source of truth: `src/client/core/src/scene-docs.ts`, `chat-docs.ts` fields listed below; render-local types for drawing/template) minus `name` (→ envelope, Task 1). Every struct: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]`, `#[ts(export, export_to = "../../types/generated/engine/")]`, `#[serde(deny_unknown_fields)]` (EXCEPT internally-tagged enums, where serde does not support it — documented limitation), `#[serde(default)]` on every optional field. Field names must serialize byte-identical to today's JSON: structs whose TS fields are camelCase get `#[serde(rename_all = "camelCase")]`; mixed-case structs (e.g. token has `actor_id` AND camel elsewhere is absent — token is all-snake) rename per-field. Core definitions:

```rust
// ---- token.rs ----
#[serde(deny_unknown_fields)]
pub struct TokenEngine {
    pub x: f64, pub y: f64, pub w: f64, pub h: f64, pub rotation: f64,
    #[serde(default)] pub visual: Option<TokenVisual>,
    #[serde(default)] pub actor_id: Option<Uuid>,
    #[serde(default)] pub overrides: Option<TokenOverrides>,
    #[serde(default)] pub face: Option<String>,
}
#[serde(deny_unknown_fields)]
pub struct TokenOverrides {
    #[serde(default)] pub name: Option<String>,
    #[serde(default)] pub visual: Option<TokenVisual>,
    #[serde(default)] pub size: Option<Size>,
    #[serde(default)] pub shape: Option<String>,       // "square" | "circle" (string in v1; literal set asserted by the battery)
    #[serde(default)] pub vision: Option<Vec<VisionAssignment>>,
}
#[serde(deny_unknown_fields)]
pub struct Size { pub w: f64, pub h: f64 }
#[serde(deny_unknown_fields)]
pub struct VisionAssignment { pub mode: String, pub range: f64 }

#[serde(tag = "kind", rename_all = "lowercase")]        // no deny_unknown_fields (serde limitation)
pub enum TokenVisual {
    Image { asset: String },
    Animated { source: AnimatedSource, fps: f64, #[serde(rename = "loop")] loop_: bool },
    Faces {
        faces: BTreeMap<String, RenderVisual>,
        default: String,
        #[serde(default, rename = "faceMap")] face_map: Option<BTreeMap<String, String>>,
    },
}
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenderVisual {
    Image { asset: String },
    Animated { source: AnimatedSource, fps: f64, #[serde(rename = "loop")] loop_: bool },
}
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AnimatedSource {
    Frames { frames: Vec<String> },
    Sheet { asset: String, rows: u32, cols: u32, #[serde(default)] count: Option<u32> },
}

#[serde(deny_unknown_fields)]
pub struct ActorEngine {
    #[serde(default)] pub display_name: Option<String>,  // rename = "displayName"
    pub visual: TokenVisual,
    pub size: Size,
    pub shape: String,
    #[serde(default)] pub faction: Option<String>,
    #[serde(default)] pub conditions: Vec<String>,
    pub prototype: bool,
    #[serde(default)] pub vision: Option<Vec<VisionAssignment>>,
}
```

```rust
// ---- scene.rs (representative; mirror scene-docs.ts:11-119 enums/defaults exactly) ----
#[serde(rename_all = "kebab-case")] pub enum MovementModel { GridStepped, Continuous }
#[serde(rename_all = "lowercase")]  pub enum MovementRestriction { Visible, Revealed, Unrestricted }
pub enum LightMode { #[serde(rename = "globalIllumination")] GlobalIllumination,
                     #[serde(rename = "environmentLight")] EnvironmentLight }
#[serde(rename_all = "lowercase")]  pub enum DiagonalRule { Chebyshev, Alternating, Euclidean, Manhattan }
pub enum EasingMode { #[serde(rename = "easeInOut")] EaseInOut, #[serde(rename = "linear")] Linear }
#[serde(deny_unknown_fields)] pub struct EnvironmentLight { pub color: String, pub intensity: f64 }
#[serde(deny_unknown_fields)] pub struct SceneDimensions { pub width: f64, pub height: f64 }
#[serde(deny_unknown_fields)] pub struct GridDistance { pub per_cell: f64, pub unit: String } // rename = "perCell"

#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneEngine {
    pub grid: Grid,
    pub background: Option<String>,
    #[serde(default)] pub bounds: Option<SceneDimensions>,
    #[serde(default)] pub snap_to_grid: Option<bool>,
    #[serde(default)] pub vision: Option<SceneVisionOverrides>,
    #[serde(default)] pub lighting: Option<SceneLightingOverrides>,
}
#[serde(deny_unknown_fields)]
pub struct Grid { pub kind: String /* "square"|"hex" */, pub size: f64,
                  #[serde(default)] pub distance: Option<GridDistance> }
// SceneVisionOverrides / SceneLightingOverrides: every field Option<...>, serde(default).
// TS declares `boolean | null` — explicit null and absent are semantically identical
// (resolvers use ??), so Option<T> with null→None is the correct mirror; a stored
// explicit null re-serializes as absent, which is semantically lossless. Document this.

#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorldSettingsEngine {
    pub scene: WorldSceneDefaults,
    pub pathfinding: Pathfinding,       // { diagonal_rule: DiagonalRule } rename diagonalRule
    pub animation: AnimationSettings,   // { speed_cells_per_sec: f64, easing: EasingMode }
    #[serde(default)] pub active_scene: Option<Uuid>,
}
// WorldSceneDefaults: all 9 fields required, camelCase — transcribe scene-docs.ts:78-88.
// impl Default for WorldSettingsEngine MUST equal client DEFAULT_WORLD_SETTINGS
// (scene-docs.ts:104-119) — add a unit test asserting the serialized default matches
// those literal values (the server-mirrors-client rule; the client constant stays the
// authoritative source, the e2e suite is the cross-check).

#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LightEngine {
    pub x: f64, pub y: f64, pub color: String, pub intensity: f64,
    pub bright_radius: f64, pub dim_radius: f64,
    #[serde(default)] pub falloff: Option<Falloff>,   // { curve: String /* linear|quadratic|none */ }
    pub enabled: bool,
}
// VisionModesEngine { modes: BTreeMap<String, VisionMode> }, VisionMode
// { id, name, illumination_floor (rename illuminationFloor), default_range, render_hint? };
// LightGradationEngine { bands: Vec<GradationBand> }, GradationBand { name, min_illumination
// (rename minIllumination) } — transcribe scene-docs.ts:454-524.
```

```rust
// ---- geometry.rs ----
#[serde(deny_unknown_fields)] pub struct Seg { pub x1: f64, pub y1: f64, pub x2: f64, pub y2: f64 }
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WallEngine { pub seg: Seg,
    #[serde(default)] pub blocks_sight: Option<bool>,
    #[serde(default)] pub blocks_light: Option<bool>,
    #[serde(default)] pub blocks_move: Option<bool> }
#[serde(deny_unknown_fields)]
pub struct RegionEngine { pub shape: RegionShape, pub behavior: String, pub cost: f64, pub enabled: bool }
#[serde(deny_unknown_fields)]
pub struct RegionShape { pub kind: String /* rect|circle|polygon */, pub points: Vec<f64> }
// DrawingEngine { shape: RegionShape-like {kind, points}, stroke {color, width}, fill? {color, alpha?} }
// TemplateEngine { shape {kind, x, y, size, direction}, color } — transcribe the exact field
// sets from render/src/drawing-view.ts:7-12 and template-view.ts:7-11 (they are the only
// authoritative shapes today; scene-tools writers must round-trip byte-identically).
```

`registries.rs`: `ChannelRegistryEngine { channels: BTreeMap<String, Channel { name } } }`, `FactionRegistryEngine { factions: BTreeMap<String, Faction { name, color, stance } } }`, `ConditionRegistryEngine { conditions: BTreeMap<String, Condition { name, icon } } }`, `ChatSettingsEngine` (six optional policy fields, transcribe `chat-docs.ts:176-183` — coordinate with the existing `ChatContentPolicy` in `chat/settings.rs`, which this REPLACES via re-export or type alias), `DiceSettingsEngine { mode, direction }` (replaces `DiceSettingsBody`).

`mod.rs` registry:

```rust
pub fn is_engine_doc_type(doc_type: &str) -> bool {
    matches!(doc_type,
        "token" | "scene" | "wall" | "region" | "light" | "drawing" | "template"
        | "actor" | "message" | "world-settings" | "vision-modes" | "light-gradation"
        | "chat-settings" | "dice-settings" | "channel-registry" | "faction-registry"
        | "condition-registry")
}

pub fn validate_engine(doc_type: &str, engine: Option<&serde_json::Value>) -> Result<(), DataError> {
    fn check<T: serde::de::DeserializeOwned>(v: &serde_json::Value, t: &str) -> Result<(), DataError> {
        serde_json::from_value::<T>(v.clone())
            .map(|_| ())
            .map_err(|e| DataError::BadEngine(format!("{t}: {e}")))
    }
    match (is_engine_doc_type(doc_type), engine) {
        (false, None) => Ok(()),
        (false, Some(_)) => Err(DataError::BadEngine(format!(
            "doc_type '{doc_type}' is not engine-defined; `engine` must be absent"))),
        (true, None) => Err(DataError::BadEngine(format!(
            "doc_type '{doc_type}' requires an `engine` body"))),
        (true, Some(v)) => match doc_type {
            "token" => check::<TokenEngine>(v, "token"),
            "scene" => check::<SceneEngine>(v, "scene"),
            "wall" => check::<WallEngine>(v, "wall"),
            "region" => check::<RegionEngine>(v, "region"),
            "light" => check::<LightEngine>(v, "light"),
            "drawing" => check::<DrawingEngine>(v, "drawing"),
            "template" => check::<TemplateEngine>(v, "template"),
            "actor" => check::<ActorEngine>(v, "actor"),
            "message" => check::<crate::chat::MessageEngine>(v, "message"),
            "world-settings" => check::<WorldSettingsEngine>(v, "world-settings"),
            "vision-modes" => check::<VisionModesEngine>(v, "vision-modes"),
            "light-gradation" => check::<LightGradationEngine>(v, "light-gradation"),
            "chat-settings" => check::<ChatSettingsEngine>(v, "chat-settings"),
            "dice-settings" => check::<DiceSettingsEngine>(v, "dice-settings"),
            "channel-registry" => check::<ChannelRegistryEngine>(v, "channel-registry"),
            "faction-registry" => check::<FactionRegistryEngine>(v, "faction-registry"),
            "condition-registry" => check::<ConditionRegistryEngine>(v, "condition-registry"),
            _ => unreachable!("is_engine_doc_type and this match must stay in sync"),
        },
    }
}

pub fn engine_of<T: serde::de::DeserializeOwned + Default>(doc: &Document) -> T {
    doc.engine.as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}
```

(Note: `message` arm forward-references Task 7's `MessageEngine` rename — implement the arm with the CURRENT struct name `MessageSystem` re-exported, and let Task 7 rename it; both tasks compile independently. Task 7 MUST add `#[serde(deny_unknown_fields)]` to the renamed `MessageEngine` — `MessageSystem` lacks it today, so until then message engine bodies do not reject unknown fields; Task 7's reviewer verifies it landed.)

- [ ] **Step 2: Unit battery** (`engine/mod.rs` tests) — for EVERY doc_type in the registry: (a) a minimal valid body deserializes; (b) an unknown field is rejected (struct-level; skip for tagged-enum-only bodies); (c) a wrong-typed field (`"x": "12"`) is rejected; (d) `validate_engine("item", Some(&json!({})))` and `validate_engine("custom-thing", Some(&json!({})))` are `Err`; (e) `validate_engine("item", None)` and `validate_engine("custom-thing", None)` are `Ok`; (f) `validate_engine("token", None)` is `Err`. For token/actor visuals: all three `TokenVisual` kinds + both `AnimatedSource` types round-trip. Literal-set assertions: every string the CLIENT writers emit today (`"square"`, `"circle"`, region `"rect"|"circle"|"polygon"`, behaviors used by scene-tools) deserializes.
- [ ] **Step 3: Run** full `cargo test`: PASS. Verify `src/types/generated/engine/` now holds the exported TS types.
- [ ] **Step 4: Commit** — `feat(m13-0): typed engine structs + validate_engine registry (data/engine)`

---

### Task 3: Ingress gate + per-block size caps + writable `/name`

**Files:**
- Modify: `src/server/src/data/validation.rs` (per-block caps)
- Modify: `src/server/src/data/command.rs` (Create/Update post-image engine validation)
- Modify: `src/server/src/data/permission.rs:27-37` (`required_cap_for_path`)
- Test: co-located module tests in each

**Interfaces:**
- Consumes: `validate_engine`, `is_engine_doc_type` (Task 2).
- Produces: every persistence path rejects invalid engine bands; `/engine/*` and `/name` are writable under `core:write_fields`.

- [ ] **Step 1: Failing tests.** In `validation.rs`: an oversized `engine` block (> 256 KiB) is rejected exactly like an oversized `system` (reuse the existing four-test pattern at `validation.rs:59-102` with `engine` in place of `system`, including the embedded-descendant cases). In `permission.rs`: `required_cap_for_path("/engine") == Some(cap::WRITE_FIELDS)`, same for `"/engine/x"`, `"/name"`; `"/engine_x"` → `None` (boundary). In `command.rs`: a Create of `doc_type: "token"` with `engine: {"x": "not-a-number", …}` errors; a Create of `doc_type: "item"` with any `engine` errors; an Update whose post-image `engine` no longer deserializes errors; an Update writing `/engine/w` with a valid value succeeds.
- [ ] **Step 2: Run — FAIL. Step 3: Implement.**
  - `validation.rs`: generalize the recursion —

```rust
/// Maximum serialized size of EACH opaque body block (`system`, `engine`).
pub const MAX_SYSTEM_BYTES: usize = 256 * 1024;   // name kept: referenced across the codebase

pub fn validate_system_size(doc: &Document) -> Result<(), DataError> {
    let sys = serde_json::to_vec(&doc.system)?.len();
    if sys > MAX_SYSTEM_BYTES { return Err(DataError::TooLarge(sys)); }
    if let Some(engine) = &doc.engine {
        let eng = serde_json::to_vec(engine)?.len();
        if eng > MAX_SYSTEM_BYTES { return Err(DataError::TooLarge(eng)); }
    }
    for children in doc.embedded.values() {
        for child in children { validate_system_size(child)?; }
    }
    Ok(())
}
```

  - `permission.rs`: extend `required_cap_for_path` with `path == "/engine" || path.starts_with("/engine/")` → `Some(cap::WRITE_FIELDS)` and `path == "/name"` → `Some(cap::WRITE_FIELDS)` (keep `/name/…` → `None`: name is a leaf).
  - `command.rs`: at every call site of `validate_system_size` (Create; Update post-image; embedded mutation paths — find them all with `grep -rn "validate_system_size" src/server/src`), additionally call a new recursive gate:

```rust
pub fn validate_engine_tree(doc: &Document) -> Result<(), DataError> {
    crate::data::engine::validate_engine(&doc.doc_type, doc.engine.as_ref())?;
    for children in doc.embedded.values() {
        for child in children { validate_engine_tree(child)?; }
    }
    Ok(())
}
```

    validating the POST-IMAGE (after all `FieldChange`s apply), so `/engine/x` writes, wholesale `/engine` replacement, and embedded-child engine writes are all covered by one chokepoint.
- [ ] **Step 4: Run** full `cargo test`. Existing fixtures whose doc_types are engine-defined but carry `system`-shaped bodies will now FAIL the gate — re-root those fixtures (`system: json!({"x": …})` → `engine: Some(json!({"x": …})), system: json!({})`) as part of this task ONLY where the failure is the gate itself; deeper read-path re-roots belong to Tasks 4–7 (if a fixture's test exercises reads not yet re-rooted, move the fixture change into that task's commit instead — keep each task green).
- [ ] **Step 5: Commit** — `feat(m13-0): strict engine ingress gate + per-block size caps + writable /name`

---

### Task 4: Redaction + search re-root (server, security-sensitive)

**Files:**
- Modify: `src/server/src/data/permission.rs` (`filter_properties` ~251-295; tests 590-1500 region)
- Modify: `src/server/src/data/search.rs` (`index_content` 29-33, `index_content_public` 41-49)
- Test: co-located

**Interfaces:**
- Consumes: Document.name/engine (Task 1).
- Produces: `/engine` and `/name` overrides redact to `null`; FTS indexes `name ∪ engine ∪ system`, visibility-partitioned.

- [ ] **Step 1: Failing tests.**
  - `filter_properties`: with `property_overrides["/engine"] = gm_only`, a non-GM recipient's doc has `engine == None` (nulled, not stripped) and the document still deserializes; with `["/name"] = owner_or_gm`, a non-owner player gets `name == None` while the owner and GM keep it (mirror the existing `/system/name` tier tests at permission.rs:734+, re-pointed); with `["/engine/vision"] = gm_only`, the `vision` key is absent from a player's engine body but `/engine/visionmode`-style boundary neighbors survive (mirror the existing 598/625 tests).
  - `collect_hidden`: an embedded child's `/engine/...` override surfaces as `/embedded/<key>/<i>/engine/...` (pointer-generic — expected to pass without code change; the test pins it).
  - `search.rs`: a doc with `name: Some("Strahd")`, engine `{"x": 3}`, system `{"bio": "vampire"}` indexes all three; with `/name` at gm_only, the public index misses "Strahd" but the GM index has it (mirror the existing partitioned-index tests).
- [ ] **Step 2: Run — FAIL. Step 3: Implement.**
  - `filter_properties`: generalize the special case —

```rust
        // `/system`, `/engine`, `/name` target Document fields directly — dropping the
        // key would (for `system`) fail re-deserialization or (for the Options) be
        // indistinguishable from a doc that never had one in a way that breaks the
        // client's stable envelope shape. Null them instead; nested pointers keep the
        // normal strip (callers rely on true key absence one level down).
        match pointer.as_str() {
            "/system" | "/engine" | "/name" => {
                if let Some(f) = whole.get_mut(&pointer[1..]) { *f = serde_json::Value::Null; }
            }
            _ => strip_pointer(&mut whole, &pointer),
        }
```

  - `search.rs`: `index_content` gains the name and engine sources — index `doc.name` as a string leaf, then recurse `doc.engine` (when Some) with the same leaf-walk as `doc.system`. `index_content_public` is unchanged in structure (it already re-runs `filter_properties` first — the nulled bands contribute nothing).
- [ ] **Step 4: Run** full `cargo test`: PASS. **Step 5: Commit** — `feat(m13-0): /engine + /name redaction (null-not-strip) + FTS over name∪engine∪system`

**⚠ Buddy-check (pre-authorized):** this task changes the per-recipient redaction chokepoint. After the task's review gate, dispatch an additional adversarial pass focused on: leak via `Update`-delta paths (`collect_hidden`), leak via FTS snippets, and the `/engine`-vs-`/engine/…` boundary.

---

### Task 5: Scene derivation reads re-root (server)

**Files:**
- Modify: `src/server/src/scene/mod.rs` (all body reads: `sys_f64` 1882-1885, `resolve_scene` 505-566, `scene_grid_sizes` 848-855, `sight_walls`/`light_walls`/`move_walls` 867-948, `region_field` 1168-1200, `scene_lights` 1208-1263, `token_vision_floors` 1293-1327, `validated_world_settings_system` 431-496, `resolved_diagonal_rule`/`resolved_animation_speed`/`resolved_bands`/`resolved_vision_modes` 579-650, `token_position` 697-699, vision-input readers 751-795)
- Modify: `src/server/src/scene/regions.rs` (`parse_region_shape` 256-282 consumes `RegionEngine`)
- Test: the existing scene/vision/region/lighting suite (re-rooted fixtures)

**Interfaces:**
- Consumes: Task 2 structs via `engine_of` / explicit `from_value`.
- Produces: all derivations read `doc.engine`; NO pointer walk into `doc.system` remains in `scene/`.

- [ ] **Step 1:** Re-root fixtures for this file's tests (`system: json!({…})` → `engine: Some(json!({…})), system: json!({})`). Run: the suite FAILS against the still-`system`-reading code.
- [ ] **Step 2: Convert each reader.** The pattern, shown complete for `resolve_scene`'s bounds/vision (apply uniformly):

```rust
// BEFORE (pointer walk):           let w = sys_f64(doc, "/bounds/width");
// AFTER (typed, fail-closed):
let scene: SceneEngine = engine_of(doc);            // ingress-validated; default on absent
let bounds = scene.bounds
    .filter(|b| b.width.is_finite() && b.height.is_finite() && b.width > 0.0 && b.height > 0.0)
    .unwrap_or(DEFAULT_SCENE_BOUNDS_UNITS);          // read-side backstop preserved verbatim
```

  Per-function requirements (semantics identical to today — the suite is the oracle):
  - `scene_grid_sizes`: `scene.grid.size`, default 100.0, reject ≤ 0.
  - `sight_walls`/`light_walls`/`move_walls`: `WallEngine`; a missing/false flag excludes the wall exactly as the pointer read did.
  - `region_field`: `RegionEngine`; disabled → dropped; behavior strings `"impassable"`/`"arrest"`/else-terrain unchanged; cost clamp max 1.0 read-side; the `/system` secrecy-tier lookup becomes `property_overrides.get("/engine")` (coordinate with the client writer change in Task 8).
  - `scene_lights`: `LightEngine`; intensity clamp 0..1 read-side; falloff curve default unchanged.
  - `token_vision_floors`: `TokenEngine` (`actor_id`, `overrides.vision`), embedded instanced actor via the CHILD's `engine` (`embedded.actor[0].engine` → `ActorEngine.vision`), linked actor via the actors map likewise.
  - `validated_world_settings_system` → rename `validated_world_settings_engine`: the structural triple guard (`scene`+`pathfinding`+`animation` all present) becomes "the doc's engine deserializes into `WorldSettingsEngine`" — same fallback to built-ins on failure. `resolved_*` helpers read the typed struct.
  - `token_position` + vision-input readers: `TokenEngine.x/.y`.
  - Delete `sys_f64` once no caller remains.
- [ ] **Step 3: Run** full `cargo test`: PASS (this file's suite green; `token_move` still red is NOT acceptable — that function is Task 6's, so if Step 2 touches shared fixtures coordinate the two tasks' commits: Task 5 must leave `cargo test` green, deferring only files untouched by it).
- [ ] **Step 4: Commit** — `refactor(m13-0): scene derivations read typed engine band`

---

### Task 6: Movement gate + token post-image re-root (server, security-critical)

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`token_move` 708-734), `src/server/src/scene/move_exec.rs`, `src/server/src/ws/room.rs` (gate consumption), `src/server/src/scene/move_stream.rs` (position reads)
- Test: the existing movement-gate/bypass suite (re-rooted) + new bypass cases

**Interfaces:**
- Consumes: `TokenEngine` (Task 2).
- Produces: the gate computes post-image `/engine/x`, `/engine/y` with the same last-write-wins + wholesale-write bypass-proofing as today's `/system` handling.

- [ ] **Step 1: Failing tests** — re-root the existing bypass suite paths (`/system/x` → `/engine/x`; wholesale `/system` → wholesale `/engine`) and ADD: (a) a `FieldChange` at `/engine` replacing the whole band with moved coordinates goes through the gate; (b) duplicate `/engine/x` changes: last write wins; (c) a `/system/x` write on a token no longer moves anything (it's game-system data now — must NOT touch the gate) and does not desync the ECS.
- [ ] **Step 2:** Re-root `token_move`'s post-image computation (`/system/x|y` literals → `/engine/x|y`, wholesale key `"/engine"`), and every position read in `move_exec.rs` / `move_stream.rs` / `room.rs` to `TokenEngine` via `engine_of`. Semantics byte-identical.
- [ ] **Step 3: Run** full `cargo test`: PASS. **Step 4: Commit** — `refactor(m13-0): movement gate + move stream on engine band`

**⚠ Buddy-check (pre-authorized):** the movement gate is a server-authoritative security boundary (server-authoritative movement rule). Adversarial pass on: gate bypass via `/engine` wholesale writes, via embedded paths, via mixed `/engine` + `/engine/x` change lists, and via `/system/x` decoys.

---

### Task 7: Chat re-root (server)

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`MessageSystem` → `MessageEngine`; every `from_value(cur.system…)` at 758, 909, tests; server-authored message Creates write `engine`, `system: {}`, envelope `name: None`)
- Modify: `src/server/src/chat/settings.rs` (`ChatContentPolicy`/`DiceSettingsBody` read `doc.engine` with the same `unwrap_or_default`; unify with Task 2's `registries.rs` types via re-export so ONE definition exists)
- Test: the chat suite (re-rooted fixtures)

- [ ] **Step 1:** Rename + re-root; fixtures move bodies to `engine`. The ingest caps (`MAX_MESSAGE_CHARS`, inline-roll caps) and `ops_target_message` ingress guard are UNCHANGED in behavior — they now read/inspect `engine`. `ops_target_message`'s block on client-authored message ops must also cover `/engine` paths (it blocked `/system` writes before; verify and re-point its path checks).
- [ ] **Step 2: Run** full `cargo test`: PASS. **Step 3: Commit** — `refactor(m13-0): chat message + settings bodies on engine band`

---

### Task 8: Client core re-root

**Files:**
- Modify: `src/client/core/src/wire.ts` (envelope: `name`, `engine`)
- Modify: `src/client/core/src/scene-docs.ts` (delete body interfaces; re-export generated engine types; re-root accessors/resolvers/writers)
- Modify: `src/client/core/src/actor.ts` (resolution engine reads `engine` + envelope `name`)
- Modify: `src/client/core/src/chat-docs.ts` (Zod guard reads `doc.engine`)
- Modify: `src/client/core/src/sheets.ts` + `src/client/ui-kit/src/sheetEdit.ts` (edit-path prefixes)
- Test: each package's existing Vitest suite (re-rooted fixtures) + `pnpm -r typecheck`

**Interfaces:**
- Consumes: `src/types/generated/engine/*.ts` (import via the same specifier pattern existing code uses for `types/generated` — grep for the current import form and match it).
- Produces: `WireDocument.name: string | null`, `WireDocument.engine?: unknown`; all accessors typed against generated `*Engine`.

- [ ] **Step 1: wire.ts** — add to `WireDocument` and `DocumentSchema`:

```ts
  name: z.string().nullable(),          // envelope; redacts to null under a /name override
  engine: z.unknown(),                  // engine band; server-validated, typed via generated *Engine
```

- [ ] **Step 2: scene-docs.ts** — delete `TokenSystem`, `SceneSystem`, `ActorSystem`, `ItemSystem`, `WorldSettingsSystem`, `LightSystem`, `RegionSystem`, `TokenOverrides`, `TokenVisual`/`RenderVisual`/`AnimatedSource`/`FaceVisual`, `VisionAssignment`, registry/system interfaces, and the settings enums IF the generated types replace them 1:1 (keep client-only helpers: `DEFAULT_WORLD_SETTINGS`, `DEFAULT_SCENE_BOUNDS`, `deepFreeze`, `resolveSceneSettings`, doc builders). Re-export the generated types under the OLD names where that keeps the diff small (`export type TokenEngine = …` + `export type { TokenEngine as TokenSystem }` is NOT allowed — rename consumers to `*Engine`; stale-name aliases defeat the sweep in Task 10). Re-root every `doc.system as X` cast to `doc.engine as XEngine`; `resolveSceneSettings` reads `ws.engine`/`scene.engine`, structural triple guard intact; `buildSceneEntityDoc` emits `{ name, engine, system: {} }`; item name reads/writes → `doc.name`; `setNameHidden` → `"/name"`; `setRegionVisibility` → `"/engine"`.
- [ ] **Step 3: actor.ts** — `resolveTokenActor`/`project`/`resolveConditions`/`resolveTokenBox`/`resolveTokenVisual`/`resolveFace` read `token.engine`/`actor.engine`; resolved NAME comes from `actor doc.name` merged under `overrides.name`; `conditionTarget` writes `/engine/conditions` (linked) or `/embedded/actor/0/engine/conditions` (instanced).
- [ ] **Step 4: chat-docs.ts** — `parseMessageSystem` → `parseMessageEngine`, parses `doc.engine`; Zod schema stays as the runtime guard, now built to match the generated `MessageEngine` type (add a compile-time `satisfies`/assignability check between the Zod inference and the generated type so drift fails `typecheck`).
- [ ] **Step 5: sheets** — `sheetEdit.ts`'s `setField` is prefix-agnostic (takes a path) — verify; `sheets.ts`'s `systemPrefix`/`SystemTreeEditor` binding stays `"/system"` (now purely game-system data); any sheet code editing engine-known fields (visual/vision/conditions editors) emits `/engine/...`.
- [ ] **Step 6: Run** `pnpm --filter @shadowcat/core test` + `pnpm -r typecheck` (typecheck is mandatory — Vitest strips types and passes type errors otherwise): PASS. **Step 7: Commit** — `refactor(m13-0): client core on envelope name + generated engine types`

---

### Task 9: Render + modules re-root

**Files:**
- Modify: `src/client/render/src/{reconciler,token-view,wall-view,region-view,drawing-view,template-view,grid}.ts`, `Stage.svelte` — DELETE the local `WallSystem`/`DrawingSystem`/`TemplateSystem`/`RegionSystemLike`/local `TokenSystem` types; import generated `*Engine`; read `doc.engine`.
- Modify: `src/modules/scene-tools/**` (`controller.svelte.ts` reads `scene.engine.grid`, `actor.engine.prototype`, `doc.engine.{x,y}`; `ToolRail.svelte` reads `scene.engine.snapToGrid`; ALL entity writers emit `{name, engine, system: {}}` bodies and `/engine/...` field changes)
- Modify: `src/modules/{stage,actors,factions,conditions,settings,chat*,sheet-*}/**` — every remaining `doc.system` read of an engine-shaped doc / every `/system/...` field-change literal targeting engine fields (find them ALL: `grep -rn -e '\.system' -e '"/system' src/client/render src/modules --include='*.ts' --include='*.svelte'` and disposition each hit: engine-band → re-root; genuine system-band (SystemTreeEditor, Nightfox-facing docs) → keep).
- Test: each package's suite + typecheck

- [ ] **Step 1:** Convert per the grep dispositions. **Step 2: Run** `pnpm -r test && pnpm -r typecheck && pnpm lint`: PASS. **Step 3:** `pnpm build` (dist for the server e2e). **Step 4: Commit** — `refactor(m13-0): render + modules on engine band`

---

### Task 10: Cross-boundary gate — e2e, stale-ref sweep, rejection battery

**Files:**
- Modify: `src/client/core/src/e2e/*.e2e.test.ts` (fixtures on the new shape)
- Create: server ingress-rejection e2e coverage (extend the existing e2e suite file set)
- No production code except fixes the gate itself surfaces.

- [ ] **Step 1: Stale-ref sweep** — whole tree, ALL file types (recorded lesson: no include-allowlists): `grep -rn '"/system' . --exclude-dir={node_modules,target,dist,.git,graphify-out}` and `grep -rn 'system\.\(x\|y\|grid\|seg\|blocks\|shape\|behavior\|visual\|vision\|conditions\|bounds\|lighting\)' . --exclude-dir=...`. Disposition EVERY hit: engine-band remnant → fix; intentional system-band (`/system` cap mapping, SystemTreeEditor, Nightfox specs/plans, permission tests exercising the system band) → leave. Record the disposition list in the task report.
- [ ] **Step 2: e2e** — re-root fixtures; run the full node↔Rust e2e suite (`pnpm --filter @shadowcat/core test:e2e`, server pre-built with dist embedded). Add e2e cases: (a) Create token with an unknown engine field → server rejects, client sees the error frame; (b) Create `item` with `engine` → rejected; (c) `/name` privacy override round-trip: owner sees name, other player receives `name: null`, FTS search by the hidden name returns nothing for the player; (d) region hidden via `/engine` + `permissions.default` behaves as before (existing region-secrecy e2e re-rooted proves it).
- [ ] **Step 3:** Full matrix locally: `cargo test` (server), `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, e2e. All green. **Step 4: Commit** — `test(m13-0): e2e re-root + ingress-rejection battery + stale-ref sweep`

---

### Task 11: Docs + skills gate (checkpoint completion)

**Files:**
- Modify: `docs/design/ARCHITECTURE.md` (§2 invariant 6: `system` authority stays structural-only; `engine` is typed, server-validated engine territory; the enumerated-geometry exception list dissolves into the band definition. Also §-wherever the document shape is described.)
- Modify: `docs/PLAN.md` (M13-0 done entry), `docs/TODO.md` (log any deferred dispositions from the sweeps), `docs/POST_WORK_FINDINGS.md` (anomalies found mid-run, if any)
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md` (three bands, `/engine` + `/name` redaction, per-block caps, `validate_engine` chokepoint), `shadowcat-codebase-scene-rendering` (typed engine reads, gate paths), `shadowcat-codebase-sheets` (`/engine` vs `/system` edit prefixes), `shadowcat-codebase-chat` (`MessageEngine`), `shadowcat-codebase-core` (ts-rs note: engine types generated; the band model)
- Modify: memory (`m13-nightfox-planning.md` resume state: M13-0 done)

- [ ] **Step 1:** Update all; dispatch `shadowcat-spec-reviewer` on the combined skill diff (reviewed skill-update gate — and per the recorded lesson, treat its PASS as necessary, not sufficient; the whole-branch final review re-checks the skill diffs).
- [ ] **Step 2: Commit** — `docs(m13-0): architecture invariant + skills + plan sync`

---

## Self-Review (spec → plan)

- Spec S1 (composition/no-engine-for-community) → Tasks 2, 3, 10b. S2 (envelope name, `/name` override, displayName stays engine) → Tasks 1, 3, 4, 8. S3 (strict ingress, fail-closed backstop) → Tasks 2, 3, 5. S4 (`modules` reserved) → Task 1 test. S5 (Zod-unknown + generated types) → Task 8. S6 (per-block caps) → Task 3. S7 (cap mapping) → Task 3. Spec §5 server changes → Tasks 3–7. §6 client → Tasks 8–9. §7 testing → Tasks 5/6 oracles + Task 10. §8 deferred items untouched. §9 → Task 11.
- Type-consistency: `validate_engine`/`engine_of`/`is_engine_doc_type` names used identically in Tasks 2/3/5/6; `MessageEngine` forward-reference between Tasks 2 and 7 is explicitly reconciled in Task 2 Step 1.

## Model/Effort directives

- Plan authored mainline on Fable 5 (standing user directive).
- Execution: **subagent-driven-development** — implementers `shadowcat-coder` (sonnet, `effort: medium`), reviewers `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (`effort: high`) per task, `-opus` twins on BLOCKED/shallow findings.
- Branch `m13-0-document-shape` in a git worktree off local **main** (the primary tree may hold the M12.5 session — do not touch it). Merge to main + customary whole-branch buddy-check at completion. No push (standing directive).

## Buddy-check directives

- **Pre-authorized:** Task 4 (redaction chokepoint) and Task 6 (movement gate) — adversarial passes as specified in each task.
- Tasks 2/3 are covered by their rejection batteries + standard two-reviewer gates; Task 10's sweep disposition list is reviewed by the final whole-branch review.
