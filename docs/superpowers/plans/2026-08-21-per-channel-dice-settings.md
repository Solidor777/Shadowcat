# Per-Channel Dice-Settings Overrides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** GM-authored per-channel overrides of the ambient dice-notation `mode`/`direction` pair — `resolve_dice_context` resolves under the SENDING channel's registered override when one exists, else the world default — closing `docs/TODO.md`'s bucket-C sub-project 5 ("Per-channel / per-message dice-settings overrides").

**Architecture:** `DiceSettingsEngine` (the existing `dice-settings` singleton config doc's engine band) gains `channel_overrides: BTreeMap<String, ChannelDiceOverride>`, a full-replacement (never partially merged) `{mode, direction}` pair keyed by `channel-registry`'s channel ids. `chat::settings::resolve_dice_context` gains a `channel: &str` parameter: a map hit resolves under that entry; a miss (including an empty map) falls back to the doc's own `mode`/`direction`; every existing fail-closed path (query error, absent doc, malformed body) is unchanged and channel-independent. `handle_send_message`'s three existing `resolve_dice_context` call sites thread the request's own `channel` (already a function parameter) through unchanged otherwise. Per-message inline notation (`t<N>`/`cs>N`) already forces `SuccessCount` regardless of any ambient setting — this plan adds one regression test proving that still holds against a per-channel-resolved ambient, per spec §1 (no new plumbing for that half). A new GM editor in `module-game-settings`'s Dice section enumerates `channel-registry`'s channels, each with an inherit/custom tri-state matching the existing world-default controls' shape.

**Tech Stack:** Rust (axum, sqlx/SQLite, serde_json, ts-rs), TypeScript (Svelte 5 runes, Vitest, `@shadowcat/core`).

**Spec:** `docs/superpowers/specs/2026-08-21-per-channel-dice-settings-design.md` (approved — every design fork below, including full-replacement-not-partial-merge and `channel`'s narrow new server-side role, is FINAL per that spec).

## Standing campaign directive

Invoke the shadowcat core skill immediately. Your goal is to close all existing bugs and to-dos within Shadowcat. The iron rule is no deferrals, of existing work, or new work as it comes up - we fix this now unless the user gives express authorization. The only exception is if a bug or to-do has a genuine blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another iron clad rule is that when faced with a design fork, determine the best long term shape in keeping with our plans and goals, and implement accordingly. You only need to ask the user if the question "what is the best long term shape in keeping with our plans and goals?" is not able to answer the question. Churn is not a concern. This paragraph must be copied verbatim to any agents dispatched in this campaign.

## Model/Effort directives

- Dispatcher: mainline session (continuing the bugs/TODOs closure campaign).
- Implementer: `shadowcat-coder` (sonnet, effort **medium**) as unnamed one-off dispatches, one task at a time.
- Per-task review: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair (sonnet, effort **high** each).
- **STANDING USER DIRECTIVE (2026-08-19, still in force): opus is BANNED for every subagent in this campaign** — no `-opus` twin dispatch of any kind (`shadowcat-coder-opus`, `shadowcat-spec-reviewer-opus`, `shadowcat-code-reviewer-opus` are all out). On a BLOCKED report or a shallow/uncertain reviewer finding, re-dispatch `shadowcat-coder`/the reviewer pair with a narrower scope or more context instead of escalating to an opus twin; escalate to the user only if a sonnet retry still can't resolve it.
- Final whole-branch review: the same `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair (sonnet, high) — not opus twins, per the standing directive above.

## Buddy-check directives

No task is buddy-check-flagged. This plan touches no untrusted-notation execution path (`dice::eval`, `chat::rolls`'s cap/execution/entropy logic) — only a config-doc lookup's added parameter and a GM-authored settings doc's shape (spec §4: misresolving `channel` "at worst changes which mode/direction a roll uses, never who can see a message"). The standard two-reviewer gate above is the review checkpoint for every task.

## Global Constraints

- **Full replacement, never partial merge** (spec §2): a `channel_overrides` entry always carries BOTH `mode` and `direction` together; no code path may write or read one field of an override independently of the other, and no "inherit this field only" semantics may be introduced.
- **`channel`'s server-side role stays narrowly scoped to dice-settings resolution** (spec §4): no code in this plan may branch on `channel` for document visibility, message audience, or any capability/authorization check. `Audience` remains the ONLY server-enforced visibility concept for chat.
- **`ChannelDiceOverride` mirrors `DiceSettingsEngine`'s own `{mode, direction}` shape exactly** (same two enum types, same field names) — one resolution semantics shared by both, never a second merge rule invented for the override.
- **ts-rs sync (CI-enforced):** any Rust engine-band type change regenerates `src/types/generated/**` via `cargo test` (server); commit the regenerated files alongside the Rust change in the same task.
- **No client-side Zod schema for `DiceSettingsEngine`/`ChannelDiceOverride`** — both ride the raw ts-rs TS type via `@shadowcat/types`, matching the existing `DiceSettingsEngine`/`ChannelRegistryEngine` precedent in `chat-docs.ts` (GM-authored config, not untrusted wire input needing runtime validation).
- **`pnpm build` before any cargo build that produces the server BINARY** (dist-embedding is a compile-time dependency of the binary target) — plain `cargo test`/`fmt`/`clippy` against the library does not need it; the final integration task's full-matrix run does.
- Comments: present-tense, invariants-first, no process/history narration, per this project's commenting rules.
- No lint suppressions (`#[allow]`/`#[expect]`/`eslint-disable`/`@ts-ignore`) without explicit user sign-off — none are anticipated by this plan.
- No debug code (`dbg!`, `println!`, `console.log`, `debugger`) in any committed diff.
- Every server test file already imports what it needs via `use super::*;` in its `mod tests` block (chat/mod.rs, chat/settings.rs) — no new top-level imports are needed for the Rust test additions below.

## File Structure

```
src/server/src/data/engine/registries.rs                 [M] ChannelDiceOverride + channel_overrides field
src/server/src/data/engine/mod.rs                         [M] re-export ChannelDiceOverride
src/types/index.ts                                        [M] export ChannelDiceOverride
src/types/generated/engine/ChannelDiceOverride.ts         [G] ts-rs generated (via `cargo test`)
src/types/generated/engine/DiceSettingsEngine.ts          [G] ts-rs regenerated (via `cargo test`)
src/modules/game-settings/src/dice-settings.test.ts       [M] fixtures gain channel_overrides: {}
src/modules/game-settings/src/seed.test.ts                [M] fixture + assertion gain channel_overrides: {}
src/client/core/src/chat-docs.ts                          [M] doc-comment example only
src/client/core/src/chat-docs.test.ts                     [M] fixture + assertion gain channel_overrides: {}
src/server/src/chat/settings.rs                           [M] resolve_dice_context channel param + tests
src/server/src/chat/mod.rs                                [M] thread channel through 3 call sites; Fixture ext; tests
src/modules/game-settings/src/GameSettingsPanel.svelte    [M] per-channel dice editor UI
src/modules/game-settings/src/dice-channel-overrides.test.ts [C] new tests
src/client/ui-kit/src/locales/en.ts                       [M] new locale keys
.claude/skills/shadowcat-codebase-chat/SKILL.md           [M] correct the "zero server-enforced meaning" claim + resolve_dice_context note
.claude/skills/shadowcat-codebase-dice/SKILL.md           [M] ChannelDiceOverride sibling note
docs/PLAN.md                                              [M] closure entry
docs/TODO.md                                              [M] remove bucket-C item 5; renumber 6-8 to 5-7
```

`[M]` modify, `[C]` create, `[G]` generated by tooling (never hand-authored).

---

### Task 1: Data model — `ChannelDiceOverride` + `DiceSettingsEngine.channel_overrides`

**Files:**
- Modify: `src/server/src/data/engine/registries.rs`
- Modify: `src/server/src/data/engine/mod.rs`
- Modify: `src/types/index.ts`
- Generated (via `cargo test`): `src/types/generated/engine/ChannelDiceOverride.ts`
- Modify (regenerated, via `cargo test`): `src/types/generated/engine/DiceSettingsEngine.ts`
- Modify: `src/modules/game-settings/src/dice-settings.test.ts`
- Modify: `src/modules/game-settings/src/seed.test.ts`
- Modify: `src/client/core/src/chat-docs.ts`
- Modify: `src/client/core/src/chat-docs.test.ts`

**Interfaces (produced — Task 2/3 consume these):**
```rust
// src/server/src/data/engine/registries.rs
pub struct ChannelDiceOverride { pub mode: DiceModeSetting, pub direction: DiceDirectionSetting }
// DiceSettingsEngine gains: pub channel_overrides: BTreeMap<String, ChannelDiceOverride>
```
```ts
// src/types/generated/engine/DiceSettingsEngine.ts (ts-rs generated)
export type DiceSettingsEngine = { mode: DiceModeSetting, direction: DiceDirectionSetting, channel_overrides: { [key in string]: ChannelDiceOverride } };
```

- [ ] **Step 1: Add `ChannelDiceOverride` and the `channel_overrides` field**

In `src/server/src/data/engine/registries.rs`, replace the `DiceSettingsEngine` struct and add `ChannelDiceOverride` immediately before it:

```rust
/// A single channel's full override of the world-default dice aggregation
/// mode + win direction (`DiceSettingsEngine.channel_overrides`'s value
/// type). Mirrors `DiceSettingsEngine`'s own `{mode, direction}` shape
/// exactly — full replacement, never a partial-field merge: an override
/// always carries BOTH fields, so a channel either fully overrides the
/// world default or (absent from the map) fully inherits it; there is no
/// "override just mode, inherit direction" state to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ChannelDiceOverride {
    /// This channel's aggregation mode, overriding `DiceSettingsEngine.mode`.
    pub mode: DiceModeSetting,
    /// This channel's win direction, overriding `DiceSettingsEngine.direction`.
    pub direction: DiceDirectionSetting,
}

/// GM-configured ambient dice-notation context (mirrors the client's
/// `DiceSettingsEngine`). `#[serde(default)]` on the struct means a partial
/// or absent body fills the rest with the safe default (Total + HighWins,
/// empty `channel_overrides`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct DiceSettingsEngine {
    /// Aggregation mode ambient dice notation resolves under (the world default).
    pub mode: DiceModeSetting,
    /// Win direction ambient dice notation resolves under (the world default).
    pub direction: DiceDirectionSetting,
    /// Per-channel full overrides, keyed by `channel-registry`'s channel id.
    /// A channel absent from this map (including every channel when the map
    /// is empty) resolves against `mode`/`direction` above — this is a
    /// full-replacement override, not a partial merge, matching how the
    /// world default itself is an unconditional pair rather than
    /// independently-optional fields (see `ChannelDiceOverride`'s doc).
    #[serde(default)]
    pub channel_overrides: BTreeMap<String, ChannelDiceOverride>,
}
```

Note: `DiceSettingsEngine`'s derive list DROPS `Copy` (a `BTreeMap` field cannot implement `Copy`); `PartialEq`/`Eq`/`Default`/`Clone` all remain valid since `ChannelDiceOverride` derives all four and `BTreeMap<String, T>` is `PartialEq`/`Eq`/`Default` whenever `T` is.

Also update the module doc comment at the top of the file (lines 1-9) to mention the new type — replace:
```rust
//! Singleton config-document engine bands: `channel-registry`,
//! `faction-registry`, `condition-registry`, `chat-settings`, `dice-settings`.
//! Field shapes mirror the client's re-exported `Channel`, `FactionStance`,
//! `Faction`, `Condition`, `ChatSettingsEngine`, and `DiceSettingsEngine`.
```
with:
```rust
//! Singleton config-document engine bands: `channel-registry`,
//! `faction-registry`, `condition-registry`, `chat-settings`, `dice-settings`.
//! Field shapes mirror the client's re-exported `Channel`, `FactionStance`,
//! `Faction`, `Condition`, `ChatSettingsEngine`, `DiceSettingsEngine`, and
//! `ChannelDiceOverride` (`DiceSettingsEngine.channel_overrides`'s value type).
```

- [ ] **Step 2: Re-export `ChannelDiceOverride`**

In `src/server/src/data/engine/mod.rs`, change:
```rust
pub use registries::{
    Channel, ChannelRegistryEngine, ChatSettingsEngine, Condition, ConditionRegistryEngine,
    DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine, Faction, FactionRegistryEngine,
    FactionStance,
};
```
to:
```rust
pub use registries::{
    Channel, ChannelDiceOverride, ChannelRegistryEngine, ChatSettingsEngine, Condition,
    ConditionRegistryEngine, DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine, Faction,
    FactionRegistryEngine, FactionStance,
};
```

- [ ] **Step 3: Run the server suite to regenerate ts-rs bindings**

Run (from `src/server/`): `cargo test`
Expected: PASS; `git status` shows a NEW file `src/types/generated/engine/ChannelDiceOverride.ts` and a MODIFIED `src/types/generated/engine/DiceSettingsEngine.ts` (gains the `channel_overrides` field and an `import type { ChannelDiceOverride } from "./ChannelDiceOverride";`).

- [ ] **Step 4: Export the new generated type from `@shadowcat/types`**

In `src/types/index.ts`, immediately after the `DiceDirectionSetting` line, add:
```ts
export type { ChannelDiceOverride } from "./generated/engine/ChannelDiceOverride";
```

- [ ] **Step 5: Fix now-required-field call sites (client typecheck)**

`channel_overrides` is a non-optional field of `DiceSettingsEngine`'s TS type — every existing `buildDiceSettingsDoc(...)` call literal missing it now fails typecheck. Fix the four sites this task owns (the fifth, in `GameSettingsPanel.svelte`'s seed effect, belongs to Task 3, which changes that file's behavior anyway):

In `src/modules/game-settings/src/dice-settings.test.ts`, change all five occurrences of
`{ mode: "total", direction: "high_wins" }` → `{ mode: "total", direction: "high_wins", channel_overrides: {} }`
and the one occurrence of
`{ mode: "success_count", direction: "low_wins" }` → `{ mode: "success_count", direction: "low_wins", channel_overrides: {} }`
(both literal shapes appear across the file's `buildDiceSettingsDoc(...)` calls — update every one).

In `src/modules/game-settings/src/seed.test.ts`, change:
```ts
      buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins" }),
```
to:
```ts
      buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }),
```

In `src/client/core/src/chat-docs.ts`, update the `buildDiceSettingsDoc` doc-comment example:
```ts
 * buildDiceSettingsDoc("00000000-0000-0000-0000-000000000001", { mode: "total", direction: "high_wins" });
```
to:
```ts
 * buildDiceSettingsDoc("00000000-0000-0000-0000-000000000001", { mode: "total", direction: "high_wins", channel_overrides: {} });
```

In `src/client/core/src/chat-docs.test.ts`, change:
```ts
  const d = buildDiceSettingsDoc("w1", { mode: "success_count", direction: "low_wins" });
  expect(d.doc_type).toBe("dice-settings");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(d.engine).toEqual({ mode: "success_count", direction: "low_wins" });
```
to:
```ts
  const d = buildDiceSettingsDoc("w1", { mode: "success_count", direction: "low_wins", channel_overrides: {} });
  expect(d.doc_type).toBe("dice-settings");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(d.engine).toEqual({ mode: "success_count", direction: "low_wins", channel_overrides: {} });
```

- [ ] **Step 6: Run the client gate**

Run: `pnpm -r typecheck && pnpm -r test`
Expected: PASS (the game-settings and core packages' existing tests still pass with the added field).

- [ ] **Step 7: Run the server gate**

Run (from `src/server/`): `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/data/engine/registries.rs src/server/src/data/engine/mod.rs \
  src/types/index.ts src/types/generated/engine/ChannelDiceOverride.ts \
  src/types/generated/engine/DiceSettingsEngine.ts \
  src/modules/game-settings/src/dice-settings.test.ts src/modules/game-settings/src/seed.test.ts \
  src/client/core/src/chat-docs.ts src/client/core/src/chat-docs.test.ts
git commit -m "feat(data): add ChannelDiceOverride and DiceSettingsEngine.channel_overrides"
```

- [ ] **Step 9: Review pair** — dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` with the diff, this task section, and the relayed gate outputs.

---

### Task 2: `resolve_dice_context` gains a channel parameter; thread through ingest

`chat/settings.rs`'s signature change and `chat/mod.rs`'s call-site update are compile-coupled (three
call sites in `mod.rs` call `resolve_dice_context`), so they are ONE task — a split here would leave
an intermediate commit that cannot compile, which is not an independently testable deliverable.

**Files:**
- Modify: `src/server/src/chat/settings.rs`
- Modify: `src/server/src/chat/mod.rs`

**Interfaces:**
- Consumes: `ChannelDiceOverride`, `DiceSettingsEngine.channel_overrides` from Task 1.
- Produces: `pub async fn resolve_dice_context(repo: &dyn Repository, world: Uuid, channel: &str) -> ParseContext`; test-only `Fixture::send_channel(&self, channel: &str, content: &str, now: i64)` and `Fixture::seed_dice_settings(&self, engine: serde_json::Value)` on `chat::mod::tests::Fixture` — available to any FUTURE test in that file, not just the ones added here.

- [ ] **Step 1: Update `resolve_dice_context`'s signature and body**

Replace the existing function:
```rust
pub async fn resolve_dice_context(repo: &dyn Repository, world: Uuid) -> ParseContext {
    let default = ParseContext::default();
    let docs = match repo.query_documents(world, DICE_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return default,
    };
    let Some(doc) = docs.into_iter().next() else {
        return default;
    };
    let body: DiceSettingsEngine = match doc.engine.and_then(|v| serde_json::from_value(v).ok()) {
        Some(b) => b,
        None => return default,
    };
    ParseContext {
        mode: match body.mode {
            DiceModeSetting::Total => ModeKind::Total,
            DiceModeSetting::SuccessCount => ModeKind::SuccessCount,
        },
        direction: match body.direction {
            DiceDirectionSetting::HighWins => Direction::HighWins,
            DiceDirectionSetting::LowWins => Direction::LowWins,
        },
    }
}
```
with:
```rust
pub async fn resolve_dice_context(
    repo: &dyn Repository,
    world: Uuid,
    channel: &str,
) -> ParseContext {
    let default = ParseContext::default();
    let docs = match repo.query_documents(world, DICE_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return default,
    };
    let Some(doc) = docs.into_iter().next() else {
        return default;
    };
    let body: DiceSettingsEngine = match doc.engine.and_then(|v| serde_json::from_value(v).ok()) {
        Some(b) => b,
        None => return default,
    };
    // A registered override for the SENDING channel wins outright (full
    // replacement, per DiceSettingsEngine.channel_overrides' doc); a
    // channel absent from the map — including every channel when the map
    // is empty — falls back to the doc's own world-default mode/direction.
    let (mode, direction) = match body.channel_overrides.get(channel) {
        Some(o) => (o.mode, o.direction),
        None => (body.mode, body.direction),
    };
    ParseContext {
        mode: match mode {
            DiceModeSetting::Total => ModeKind::Total,
            DiceModeSetting::SuccessCount => ModeKind::SuccessCount,
        },
        direction: match direction {
            DiceDirectionSetting::HighWins => Direction::HighWins,
            DiceDirectionSetting::LowWins => Direction::LowWins,
        },
    }
}
```

Update the function's doc comment (immediately above it) from:
```rust
/// Read the world's ambient dice-notation `ParseContext`, fail-closed. A query
/// error, an absent `dice-settings` doc, or an `engine` body that fails to
/// deserialize into `DiceSettingsEngine` all yield `ParseContext { mode: Total,
/// direction: HighWins }` — the same safe baseline `resolve_content_policy`
/// uses for chat enrichment.
```
to:
```rust
/// Read the world's ambient dice-notation `ParseContext` for `channel`,
/// fail-closed. A query error, an absent `dice-settings` doc, or an `engine`
/// body that fails to deserialize into `DiceSettingsEngine` all yield
/// `ParseContext { mode: Total, direction: HighWins }` regardless of
/// `channel` — the same safe baseline `resolve_content_policy` uses for chat
/// enrichment. When the doc IS present and well-formed, `channel_overrides`
/// is checked first: a channel with a registered override resolves under
/// that override's `mode`/`direction` (full replacement, never merged with
/// the world default); a channel absent from the map falls back to the
/// doc's own `mode`/`direction`.
```

- [ ] **Step 2: Update the 8 existing call sites to pass a channel**

Add a channel argument to every existing `resolve_dice_context(&repo, world_id[, 2], ...)` call in this file's test module (these test the WORLD-DEFAULT / fail-closed paths, which must behave identically regardless of which channel is passed — `"general"` is used throughout as a channel that carries no override in these fixtures):

- `default_dice_context_is_total_high_wins`: unchanged (constructs `ParseContext::default()` directly, no call site here).
- Rename `absent_dice_settings_doc_resolves_to_default` to `absent_dice_settings_doc_resolves_to_default_regardless_of_channel` and replace its body:
```rust
#[tokio::test]
async fn absent_dice_settings_doc_resolves_to_default_regardless_of_channel() {
    let (repo, world_id, _gm) = world().await;
    for channel in ["general", "ic"] {
        let ctx = resolve_dice_context(&repo, world_id, channel).await;
        assert_eq!(ctx.mode, ModeKind::Total, "channel={channel}");
        assert_eq!(ctx.direction, Direction::HighWins, "channel={channel}");
    }
}
```
- Rename `malformed_dice_settings_body_resolves_to_default` to `malformed_dice_settings_body_resolves_to_default_regardless_of_channel` and replace its body:
```rust
#[tokio::test]
async fn malformed_dice_settings_body_resolves_to_default_regardless_of_channel() {
    let (repo, world_id, gm) = world().await;
    // `mode` is a type mismatch (number, not a known string), so
    // deserialization into `DiceSettingsEngine` errors outright. Seeded
    // via `seed_document_unvalidated` (raw insert) — both `apply_intent`
    // and `apply_command` would reject this Create.
    let doc = dice_settings_doc(world_id, gm, serde_json::json!({ "mode": 5 }));
    seed_settings_doc(&repo, world_id, gm, doc).await;
    for channel in ["general", "ic"] {
        let ctx = resolve_dice_context(&repo, world_id, channel).await;
        assert_eq!(ctx.mode, ModeKind::Total, "channel={channel}");
        assert_eq!(ctx.direction, Direction::HighWins, "channel={channel}");
    }
}
```
- `unknown_enum_variant_string_resolves_to_default`: change `resolve_dice_context(&repo, world_id).await` to `resolve_dice_context(&repo, world_id, "general").await`.
- `total_high_wins_is_read`, `total_low_wins_is_read`, `success_count_high_wins_is_read`, `success_count_low_wins_is_read`: each changes `resolve_dice_context(&repo, world_id).await` to `resolve_dice_context(&repo, world_id, "general").await`.
- `partial_body_defaults_the_other_field`: change both `resolve_dice_context(&repo, world_id).await` and `resolve_dice_context(&repo2, world_id2).await` to `resolve_dice_context(&repo, world_id, "general").await` and `resolve_dice_context(&repo2, world_id2, "general").await` respectively.

- [ ] **Step 3: Add the channel-override resolution tests**

Add these two new tests after `partial_body_defaults_the_other_field`:
```rust
#[tokio::test]
async fn channel_with_override_resolves_to_it() {
    let (repo, world_id, gm) = world().await;
    let doc = dice_settings_doc(
        world_id,
        gm,
        serde_json::json!({
            "mode": "total", "direction": "high_wins",
            "channel_overrides": {
                "ic": { "mode": "success_count", "direction": "low_wins" }
            }
        }),
    );
    seed_settings_doc(&repo, world_id, gm, doc).await;
    let ctx = resolve_dice_context(&repo, world_id, "ic").await;
    assert_eq!(ctx.mode, ModeKind::SuccessCount);
    assert_eq!(ctx.direction, Direction::LowWins);
}

#[tokio::test]
async fn channel_absent_from_map_falls_back_to_world_default() {
    let (repo, world_id, gm) = world().await;
    let doc = dice_settings_doc(
        world_id,
        gm,
        serde_json::json!({
            "mode": "total", "direction": "high_wins",
            "channel_overrides": {
                "ic": { "mode": "success_count", "direction": "low_wins" }
            }
        }),
    );
    seed_settings_doc(&repo, world_id, gm, doc).await;
    // "ooc" carries no override, so it resolves against the world default
    // (Total/HighWins here) despite "ic" having a DIFFERENT override
    // registered in the very same doc — proves the lookup is per-channel,
    // not "any override present anywhere widens every channel".
    let ctx = resolve_dice_context(&repo, world_id, "ooc").await;
    assert_eq!(ctx.mode, ModeKind::Total);
    assert_eq!(ctx.direction, Direction::HighWins);
}
```

- [ ] **Step 4: Run `chat::settings`'s tests to verify**

Run (from `src/server/`): `cargo test settings::tests --lib`
Expected: all `chat::settings::tests` PASS, including the two new tests and the two renamed ones.

- [ ] **Step 5: Thread `channel` through the three `resolve_dice_context` call sites in `chat/mod.rs`**

In `src/server/src/chat/mod.rs`, inside `handle_send_message`:
```rust
    let mut content_segments = if parsed.kind == MessageKind::Roll {
        let dice_ctx = resolve_dice_context(repo, room.world_id).await;
```
becomes:
```rust
    let mut content_segments = if parsed.kind == MessageKind::Roll {
        let dice_ctx = resolve_dice_context(repo, room.world_id, &channel).await;
```
and the two occurrences of:
```rust
                            dice_ctx = Some(resolve_dice_context(repo, room.world_id).await);
```
(one in the `Inline` arm, one in the `Button` arm of the `scan_body` match) both become:
```rust
                            dice_ctx = Some(resolve_dice_context(repo, room.world_id, &channel).await);
```
`channel` is already a `String` parameter of `handle_send_message`, borrowed here and moved later (unchanged) into `MessageDraft { channel, .. }` — these three borrows all end before that later move, so no ownership conflict is introduced.

- [ ] **Step 6: Add `Fixture::send_channel` and `Fixture::seed_dice_settings`**

In `src/server/src/chat/mod.rs`'s `mod tests`, inside `impl Fixture`, add these two methods immediately after the existing `send` method:
```rust
        /// Same as `send`, but to an explicit `channel` rather than the
        /// hardcoded `"all"` — needed to exercise per-channel dice-settings
        /// resolution, which `send` alone cannot reach.
        async fn send_channel(
            &self,
            channel: &str,
            content: &str,
            now: i64,
        ) -> Result<Command, SendMessageError> {
            handle_send_message(
                MessageRequestCtx {
                    room: &self.room,
                    repo: &self.repo,
                    ctx: &self.ctx,
                    rate: &self.rate,
                    preview: LinkPreviewDeps {
                        client: &self.preview_client,
                        cache: &self.preview_cache,
                        rate: &self.preview_rate,
                    },
                    now,
                    budget_per_min: 60,
                },
                channel.into(),
                content.into(),
                None,
                Audience::Public,
            )
            .await
        }

        /// Seeds a `dice-settings` doc with the given `engine` JSON via the
        /// test-only raw insert (`SqliteRepository::seed_document_unvalidated`)
        /// — `Fixture` retains no GM `PermissionContext` to drive a normal
        /// `apply_intent` Create, and a well-formed `channel_overrides` body
        /// doesn't need ingress validation to exercise `handle_send_message`'s
        /// channel-threading. `owner` must be a real created user (an FK), so
        /// this uses the fixture's own player id.
        async fn seed_dice_settings(&self, engine: serde_json::Value) {
            let doc = Document {
                id: Uuid::new_v4(),
                scope: Scope::World {
                    world_id: self.room.world_id,
                },
                doc_type: DICE_SETTINGS_DOC_TYPE.to_string(),
                schema_version: 1,
                name: None,
                source: None,
                base: None,
                owner: Some(self.ctx.user_id),
                permissions: PermissionSet::default(),
                embedded: BTreeMap::new(),
                parent_id: None,
                engine: Some(engine),
                system: serde_json::json!({}),
                created_at: 0,
                updated_at: 0,
            };
            self.repo.seed_document_unvalidated(&doc).await.unwrap();
        }
```

- [ ] **Step 7: Write the channel-threading test (proves the wiring, not just the resolver)**

Add near the other roll-related tests (e.g. after `roll_message_never_gets_a_link_preview_even_when_previews_enabled`):
```rust
    /// Ingest-level pin: `handle_send_message` resolves the ambient dice
    /// context under the SENDING channel, not a hardcoded/ignored one. A
    /// bare `t<N>` target (mode-agnostic notation — resolves to
    /// `TotalConfig.difficulty` under Total-ambient, or a SuccessCount
    /// target under SuccessCount-ambient, per the notation parser's
    /// `ParseContext`-driven resolution) sent to "ic" (which carries a
    /// SuccessCount channel override) yields `successes: Some(_)`, while the
    /// SAME formula sent to "general" (no override, world default Total)
    /// yields `successes: None`.
    #[tokio::test]
    async fn ambient_mode_resolves_per_sending_channel() {
        let f = Fixture::new(ChatContentPolicy::default()).await;
        f.seed_dice_settings(serde_json::json!({
            "mode": "total", "direction": "high_wins",
            "channel_overrides": {
                "ic": { "mode": "success_count", "direction": "high_wins" }
            }
        }))
        .await;

        let ic_cmd = f.send_channel("ic", "/roll 4d6t3", 1).await.unwrap();
        let ic_sys = f.stored_engine(&ic_cmd).await;
        let Segment::RollEmbed {
            outcome: ic_outcome,
            ..
        } = ic_sys.content.first().unwrap()
        else {
            panic!("expected a RollEmbed segment: {:?}", ic_sys.content);
        };
        assert!(
            ic_outcome.successes.is_some(),
            "channel \"ic\" carries a SuccessCount override, so a bare t<N> \
             target must resolve as a success count: {ic_outcome:?}"
        );

        let general_cmd = f.send_channel("general", "/roll 4d6t3", 2).await.unwrap();
        let general_sys = f.stored_engine(&general_cmd).await;
        let Segment::RollEmbed {
            outcome: general_outcome,
            ..
        } = general_sys.content.first().unwrap()
        else {
            panic!("expected a RollEmbed segment: {:?}", general_sys.content);
        };
        assert!(
            general_outcome.successes.is_none(),
            "channel \"general\" carries no override, so it must fall back \
             to the Total world default: {general_outcome:?}"
        );
    }
```

- [ ] **Step 8: Write the per-message-notation-precedence regression test (spec §1)**

Add immediately after the test above:
```rust
    /// Regression pin for spec §1: an explicit `cs>=N` (or `t<N>`) notation
    /// override already forces `SuccessCount` regardless of the AMBIENT
    /// resolved dice-settings — per-message overrides are fully satisfied by
    /// existing parser precedence, with no new plumbing needed for them.
    /// This re-asserts that precedence now that ambient resolution is
    /// per-channel, not just per-world: an explicit `cs>=3` still wins even
    /// though the sending channel's own override says Total.
    #[tokio::test]
    async fn inline_success_rule_notation_forces_success_count_despite_a_total_channel_override() {
        let f = Fixture::new(ChatContentPolicy::default()).await;
        f.seed_dice_settings(serde_json::json!({
            "mode": "total", "direction": "high_wins",
            "channel_overrides": {
                "ic": { "mode": "total", "direction": "high_wins" }
            }
        }))
        .await;

        let cmd = f.send_channel("ic", "/roll 4d6cs>=3", 1).await.unwrap();
        let sys = f.stored_engine(&cmd).await;
        assert_eq!(sys.kind, MessageKind::Roll);
        let Segment::RollEmbed { outcome, .. } = sys.content.first().unwrap() else {
            panic!("expected a RollEmbed segment: {:?}", sys.content);
        };
        assert!(
            outcome.successes.is_some(),
            "explicit cs>=N notation must force SuccessCount (successes \
             populated) regardless of the channel's Total-mode ambient \
             override: {outcome:?}"
        );
    }
```

- [ ] **Step 9: Run `chat::mod`'s tests to verify**

Run (from `src/server/`): `cargo test chat::tests --lib`
Expected: all `chat::tests` PASS, including the two new tests.

- [ ] **Step 10: Run the full server gate**

Run (from `src/server/`): `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/server/src/chat/settings.rs src/server/src/chat/mod.rs
git commit -m "feat(chat): resolve_dice_context resolves per-channel dice-settings overrides; thread through ingest"
```

- [ ] **Step 12: Review pair** — dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` with the diff, this task section, and the relayed gate outputs.

---

### Task 3: GM per-channel dice editor UI

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Modify: `src/modules/game-settings/src/seed.test.ts` (assertion only — the seed effect's engine literal changes)
- Create: `src/modules/game-settings/src/dice-channel-overrides.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts`

**Interfaces:**
- Consumes: `DiceSettingsEngine`, `ChannelRegistryEngine`, `buildDiceSettingsDoc`, `buildChannelRegistryDoc` from `@shadowcat/core` (all pre-existing except `DiceSettingsEngine`'s new `channel_overrides` field from Task 1).
- Produces: no new exports — this task is UI-only, wired to the existing `set()` JSON-pointer helper already in `GameSettingsPanel.svelte`.

- [ ] **Step 1: Seed effect gains `channel_overrides: {}`**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, change:
```ts
    if (ctx.documents.query("dice-settings").length === 0) {
      ops.push({ op: "create" as const, doc: buildDiceSettingsDoc(ctx.world, { mode: "total", direction: "high_wins" }) });
    }
```
to:
```ts
    if (ctx.documents.query("dice-settings").length === 0) {
      ops.push({ op: "create" as const, doc: buildDiceSettingsDoc(ctx.world, { mode: "total", direction: "high_wins", channel_overrides: {} }) });
    }
```

In `src/modules/game-settings/src/seed.test.ts`, change:
```ts
    expect(diceOp.doc.engine).toEqual({ mode: "total", direction: "high_wins" });
```
to:
```ts
    expect(diceOp.doc.engine).toEqual({ mode: "total", direction: "high_wins", channel_overrides: {} });
```

- [ ] **Step 2: Add the `channel-registry` read + `ChannelRegistryEngine` import**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, add `type ChannelRegistryEngine` to the `@shadowcat/core` import:
```ts
  import {
    buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc, buildDiceSettingsDoc,
    buildChatSettingsDoc,
    DEFAULT_WORLD_SETTINGS,
    type WorldSettingsEngine, type LightGradationEngine, type VisionModesEngine,
    type SceneEngine, type WireDocument, DEFAULT_SCENE_BOUNDS, type DiceSettingsEngine,
    type ChatSettingsEngine, type ChannelRegistryEngine,
  } from "@shadowcat/core";
```

Immediately after the `dicesys` derived (which already exists), add:
```ts
  // Read-only: this panel enumerates channel-registry's channels for the
  // per-channel dice editor below but never creates/edits the registry
  // itself (ChatPanel owns that seed/CRUD; see the chat module).
  const channelRegDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("channel-registry")[0];
  });
  const channelEntries = $derived.by((): [string, { name: string }][] => {
    const sys = channelRegDoc?.engine as ChannelRegistryEngine | undefined;
    return Object.entries(sys?.channels ?? {});
  });
```

- [ ] **Step 3: Render the per-channel override rows inside the existing Dice fieldset**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, inside the `{#if ctx.role === "gm" && dicesys && diceDoc}` Dice `<fieldset>` block, immediately before its closing `</fieldset>` (i.e. right after the existing direction `<label>` block), add:
```svelte
      {#if channelEntries.length > 0}
        <div>
          <span>{ctx.t("gameSettings.dice.channelOverrides")}</span>
          {#each channelEntries as [id, channel] (id)}
            {@const override = dicesys.channel_overrides[id]}
            <div>
              <span>{channel.name}</span>
              <label>
                {ctx.t("gameSettings.dice.channelOverride")}
                <select aria-label="gameSettings.dice.channelOverride.{id}"
                  value={override != null ? "override" : ""}
                  onchange={(e) => {
                    const v = (e.currentTarget as HTMLSelectElement).value;
                    if (v === "") {
                      const next = { ...dicesys.channel_overrides };
                      delete next[id];
                      set(diceDoc.id, "/engine/channel_overrides", dicesys.channel_overrides, next);
                    } else {
                      set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: dicesys.mode, direction: dicesys.direction });
                    }
                  }}>
                  <option value="">{ctx.t("gameSettings.inherit")}</option>
                  <option value="override">{ctx.t("gameSettings.dice.channelOverrideCustom")}</option>
                </select>
              </label>
              {#if override != null}
                <label>
                  {ctx.t("gameSettings.dice.mode")}
                  <select aria-label="gameSettings.dice.channelOverride.{id}.mode" value={override.mode}
                    onchange={(e) => set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: (e.currentTarget as HTMLSelectElement).value, direction: override.direction })}>
                    {#each DICE_MODE as m}
                      <option value={m}>{m === "total" ? ctx.t("gameSettings.dice.modeTotal") : ctx.t("gameSettings.dice.modeSuccess")}</option>
                    {/each}
                  </select>
                </label>
                <label>
                  {ctx.t("gameSettings.dice.direction")}
                  <select aria-label="gameSettings.dice.channelOverride.{id}.direction" value={override.direction}
                    onchange={(e) => set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: override.mode, direction: (e.currentTarget as HTMLSelectElement).value })}>
                    {#each DICE_DIRECTION as d}
                      <option value={d}>{d === "high_wins" ? ctx.t("gameSettings.dice.directionHigh") : ctx.t("gameSettings.dice.directionLow")}</option>
                    {/each}
                  </select>
                </label>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
```

- [ ] **Step 4: Add the two new locale keys**

In `src/client/ui-kit/src/locales/en.ts`, immediately after the existing `"gameSettings.dice.directionLow": "Low wins",` line, add:
```ts
  "gameSettings.dice.channelOverrides": "Channel overrides",
  "gameSettings.dice.channelOverride": "Custom settings",
  "gameSettings.dice.channelOverrideCustom": "Custom",
```

- [ ] **Step 5: Write the tests**

Create `src/modules/game-settings/src/dice-channel-overrides.test.ts`:
```ts
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildDiceSettingsDoc, buildChannelRegistryDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("per-channel dice-settings editor", () => {
  it("renders nothing when the channel registry has no channels", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", {}, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    // setAppContextForTest's default `t` returns the literal key (not
    // translated copy), so the section heading renders as this exact string.
    expect(screen.queryByText("gameSettings.dice.channelOverrides")).toBeNull();
  });

  it("renders one row per registered channel, defaulting to inherit", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" }, ic: { name: "In Character" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const generalSel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    const icSel = screen.getByLabelText("gameSettings.dice.channelOverride.ic") as HTMLSelectElement;
    expect(generalSel.value).toBe("");
    expect(icSel.value).toBe("");
    expect(screen.queryByLabelText("gameSettings.dice.channelOverride.general.mode")).toBeNull();
  });

  it("selecting Custom seeds mode/direction from the world default and dispatches a create", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "override" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "dice1", changes: [{ path: "/engine/channel_overrides/general", old: null, new: { mode: "total", direction: "high_wins" } }] },
    ]);
  });

  it("editing mode on an existing override writes the FULL override object (full replacement)", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const modeSel = screen.getByLabelText("gameSettings.dice.channelOverride.general.mode") as HTMLSelectElement;
    await fireEvent.change(modeSel, { target: { value: "success_count" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "success_count", direction: "high_wins" },
        }],
      },
    ]);
  });

  it("editing direction on an existing override writes the FULL override object", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const dirSel = screen.getByLabelText("gameSettings.dice.channelOverride.general.direction") as HTMLSelectElement;
    await fireEvent.change(dirSel, { target: { value: "low_wins" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "total", direction: "low_wins" },
        }],
      },
    ]);
  });

  it("switching back to Inherit removes the key via a whole-map replace", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "success_count", direction: "low_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides",
          old: { general: { mode: "success_count", direction: "low_wins" } },
          new: {},
        }],
      },
    ]);
  });

  it("is not rendered for a non-GM", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    expect(screen.queryByLabelText("gameSettings.dice.channelOverride.general")).toBeNull();
  });
});
```

- [ ] **Step 6: Run the tests to verify**

Run: `pnpm --filter @shadowcat/module-game-settings test`
Expected: all tests PASS, including the new `dice-channel-overrides.test.ts` file and the updated `seed.test.ts` assertion.

- [ ] **Step 7: Run the full client gate**

Run: `pnpm -r typecheck && pnpm -r test && pnpm lint`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/modules/game-settings/src/GameSettingsPanel.svelte src/modules/game-settings/src/seed.test.ts \
  src/modules/game-settings/src/dice-channel-overrides.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(game-settings): per-channel dice-settings override editor"
```

- [ ] **Step 9: Review pair** — dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` with the diff, this task section, and the relayed gate outputs.

---

### Task 4: Documentation sync, skill-update gate, full verification, merge

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-dice/SKILL.md`
- Modify: `docs/PLAN.md`
- Modify: `docs/TODO.md`

- [ ] **Step 1: Correct `shadowcat-codebase-chat`'s "zero server-enforced meaning" claim (spec §4)**

In `.claude/skills/shadowcat-codebase-chat/SKILL.md`, in the `MessageEngine` bullet's `Audience` sub-bullet, change:
```markdown
  - `Audience` (`Public`/`Whisper{recipients: Vec<Uuid>}`/`GmOnly`, `#[default] Public`, tagged
    enum, ts-rs exported same as `ActorOwnerRef`) — the intended readership of a message, carried
    on the `SendMessage` frame and stored verbatim in `MessageEngine`. This is the ONLY
    server-enforced visibility concept for chat; `channel` is a purely client-chosen label with
    ZERO server-enforced meaning — the server never validates or branches on it. A client module
    choosing to post to a "GM" channel is what sets `audience: GmOnly`; the server has no concept
    of a reserved channel name.
```
to:
```markdown
  - `Audience` (`Public`/`Whisper{recipients: Vec<Uuid>}`/`GmOnly`, `#[default] Public`, tagged
    enum, ts-rs exported same as `ActorOwnerRef`) — the intended readership of a message, carried
    on the `SendMessage` frame and stored verbatim in `MessageEngine`. This is the ONLY
    server-enforced VISIBILITY concept for chat; `channel` is a client-chosen label the server
    never uses to gate document visibility, message audience, or any capability check — that
    boundary is `Audience` alone. `channel` has exactly ONE narrow server-enforced reason to be
    read: `chat::settings::resolve_dice_context` looks it up against the world's `dice-settings`
    `channel_overrides` map to select which `mode`/`direction` pair an ambient roll resolves under
    (see the Dice wire section below) — misresolving it at worst changes which dice settings a
    roll uses, never who can see a message. A client module choosing to post to a "GM" channel is
    what sets `audience: GmOnly`; the server has no concept of a reserved channel name.
```

Then in the "Dice wire" section's `handle_send_message` roll-stage bullet, change:
```markdown
  Button chunks validate-only. Ambient `ParseContext` = `resolve_dice_context` (the
  `dice-settings` config doc, fail-closed Total/HighWins, GM-authored in
  module-game-settings' Dice section).
```
to:
```markdown
  Button chunks validate-only. Ambient `ParseContext` = `resolve_dice_context(repo, world,
  channel)` (the `dice-settings` config doc: a `channel_overrides` entry for the SENDING channel
  wins outright over the doc's own `mode`/`direction`, full replacement never partial merge; fail-
  closed to Total/HighWins on any query error, absent doc, or malformed body, REGARDLESS of
  channel; GM-authored in module-game-settings' Dice section, including a per-channel editor
  enumerating `channel-registry`'s channels).
```

- [ ] **Step 2: Add `ChannelDiceOverride` note to `shadowcat-codebase-dice`**

In `.claude/skills/shadowcat-codebase-dice/SKILL.md`'s orientation paragraph, change:
```markdown
Ambient `ParseContext` for chat rolls comes from the world's `dice-settings` config doc
(`chat::settings::resolve_dice_context`, fail-closed
Total/HighWins, GM-authored in `module-game-settings`'s Dice section).
```
to:
```markdown
Ambient `ParseContext` for chat rolls comes from the world's `dice-settings` config doc
(`chat::settings::resolve_dice_context`, channel-scoped: a `DiceSettingsEngine.channel_overrides`
entry for the sending channel wins over the doc's own `mode`/`direction`; fail-closed to
Total/HighWins on any query/parse failure regardless of channel; GM-authored in
`module-game-settings`'s Dice section). `ChannelDiceOverride` (`data::engine::registries`) is a
sibling type of `DiceSettingsEngine` in the world-settings-doc family — identical `{mode,
direction}` shape, so a channel's override and the document's own world default share ONE
resolution semantics, never a second merge rule.
```

- [ ] **Step 3: `docs/TODO.md` — remove the resolved bucket-C item, renumber**

In `docs/TODO.md`'s "Follow-on feature sub-projects" list, remove item 5 (`**Per-channel / per-message dice-settings overrides** — needs a channel model.`) and renumber the remaining items 6→5, 7→6, 8→7:
```markdown
1. **Recalc-from-chat** — persist `spec`/`raws` on `RollEmbed` (persistence + secrecy fork).
2. **Link-preview extensions** — server-fetch-cache-as-asset **image** pipeline + async
   post-publish enrichment (`WriteOrigin` path) + **shared preview cache** + **oEmbed** provider
   embeds (user opted both edge items in; oEmbed carries SSRF/privacy surface → threat-model it).
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Dice-notation grammar growth** — math fns (floor/ceil/round/abs/min/max) + crit-event /
   tier-ladder notation syntax.
5. **In-body doc-link chat segment** (`Segment::DocLink`) — actor-name → sheet navigation shipped
   in M12c, but a free-form doc-link segment has no server producer or client authoring path yet;
   needs a server producer + authoring affordance.
6. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` is REJECTED at ingest (fail-closed,
   no first-party producer) — build the composer/token-context UX and lift the rejection together.
7. **Real-time per-recipient move-streaming** — `MoveStream` precomputes each move's
   per-recipient vision clip at execute time, so two tokens moving simultaneously do not reveal
   each other mid-walk when a watcher's vision opens after the clip; it reconciles only at the
   stop + next `vision` rebroadcast. No correctness/secrecy impact today — only a missed
   transient reveal. Needs a per-move server loop recomputing each recipient's visibility of
   every concurrently moving token as positions advance, replacing execute-time precompute.
```

- [ ] **Step 4: `docs/PLAN.md` — closure entry**

In `docs/PLAN.md`, immediately before the `## Phase 2 — Full table` heading (right after Phase 1b's closing paragraph), add:
```markdown
### Bucket C · Per-channel dice-settings overrides ✅
**COMPLETE.** Closed bucket-C sub-project 5 (`docs/TODO.md`): `DiceSettingsEngine` gained
`channel_overrides: BTreeMap<String, ChannelDiceOverride>` — a full-replacement (never
partially-merged) `{mode, direction}` pair keyed by `channel-registry`'s channel ids.
`chat::settings::resolve_dice_context` gained a `channel: &str` parameter: a registered override
for the sending channel wins outright; a channel absent from the map, an absent/malformed
`dice-settings` doc, or a query error all fall back to (or stay on) the existing world-default/
fail-closed baseline, unchanged and channel-independent. `handle_send_message`'s three existing
call sites thread the request's own `channel` through with no other behavior change. Per-message
inline notation (`t<N>`/explicit `cs>=N`/`cf<N`) already forced `SuccessCount` regardless of any
ambient setting — re-pinned by a new regression test against a per-channel-resolved ambient, per
spec §1 (no new plumbing needed for that half). A new GM editor in `module-game-settings`'s Dice
section enumerates `channel-registry`'s channels with an inherit/custom tri-state per row,
matching the existing world-default controls' shape. `channel`'s server-side role stays narrowly
scoped to this one resolution decision — it still never gates document visibility, message
audience, or any capability check; `shadowcat-codebase-chat`'s prior "zero server-enforced
meaning" claim is corrected to state this one exception. Design:
[`superpowers/specs/2026-08-21-per-channel-dice-settings-design.md`](superpowers/specs/2026-08-21-per-channel-dice-settings-design.md).
Plan:
[`superpowers/plans/2026-08-21-per-channel-dice-settings.md`](superpowers/plans/2026-08-21-per-channel-dice-settings.md).
```

- [ ] **Step 5: Reviewed skill-update gate**

Dispatch `shadowcat-spec-reviewer` to confirm the two skill diffs (Step 1-2) accurately capture the change with no omission, drift, or broken pointer, per this project's mandatory doc-sync gate.

- [ ] **Step 6: `graphify update .`**

Run: `graphify update .`
Expected: the graph index picks up `ChannelDiceOverride`/`DiceSettingsEngine.channel_overrides`/the new `GameSettingsPanel.svelte` seam (AST-only, no API cost).

- [ ] **Step 7: Commit the docs**

```bash
git add .claude/skills/shadowcat-codebase-chat/SKILL.md .claude/skills/shadowcat-codebase-dice/SKILL.md \
  docs/PLAN.md docs/TODO.md
git commit -m "docs: close bucket-C per-channel dice-settings overrides; sync chat + dice skills"
```

- [ ] **Step 8: Full local matrix**

Run, in order (client build first — embed ordering):
1. `pnpm build`
2. from `src/server/`: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
3. `pnpm -r typecheck && pnpm -r test && pnpm lint`

Expected: all green.

- [ ] **Step 9: Final whole-branch review**

Dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (sonnet, high — NOT opus, per the standing directive) with the full branch diff (merge-base `main`..HEAD), this whole plan, and the design spec. Fix any findings via a scoped `shadowcat-coder` re-dispatch (never a mainline edit), re-verify the local matrix, then merge the feature branch into `main` and push, per this project's autonomous-commit/milestone-push rule (a full bucket-C sub-project is a milestone unit).

## Self-review

- **Spec coverage:** §1 (per-message notation regression) → Task 2 Step 8; §2 (data model) → Task 1; §3 (resolution) → Task 2; §4 (channel's narrow server role + skill correction) → Task 4 Steps 1-2; §5 (GM UI) → Task 3; §6 (testing + docs) → Task 2 (tests) + Task 4 (skills); §7 (non-goals: channel-registry itself untouched, no per-message settings UI) → honored by omission (no task modifies channel-registry's own CRUD or adds per-message UI). No gaps found.
- **Placeholder scan:** every step carries the exact code/diff text an implementer applies; no "TBD"/"similar to Task N"/"add appropriate handling" phrasing anywhere; the one tooling-generated artifact (`ChannelDiceOverride.ts`) is explicitly marked `[G]` and its exact expected shape is given, not its literal bytes (ts-rs owns that).
- **Type consistency:** `ChannelDiceOverride{mode: DiceModeSetting, direction: DiceDirectionSetting}` (Task 1) is the exact shape read by `resolve_dice_context` (Task 2: `body.channel_overrides.get(channel)` → `(o.mode, o.direction)`) and written by the UI (Task 3: `{ mode: dicesys.mode, direction: dicesys.direction }` / `{ mode: ..., direction: override.direction }`). `resolve_dice_context(repo: &dyn Repository, world: Uuid, channel: &str) -> ParseContext` (Task 2 Step 1) matches every call site added later in the SAME task verbatim (`&channel` at all three `handle_send_message` sites; `channel`/literal strings in the new Fixture methods and tests). `Fixture::send_channel`/`Fixture::seed_dice_settings` (Task 2 Step 6) are used with identical names/signatures in both new tests within the same task. `DICE_SETTINGS_DOC_TYPE` (Task 2's `seed_dice_settings`) is the pre-existing re-export already in scope via `use super::*;` (Global Constraints note) — no new import needed.
