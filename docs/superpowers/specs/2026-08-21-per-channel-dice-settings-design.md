# Per-Channel Dice-Settings Overrides — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority).

**Spec for:** `docs/TODO.md` bucket-C sub-project 5, "Per-channel / per-message dice-settings
overrides — needs a channel model."

**Corrects the TODO's own premise:** a channel model already exists and ships today
(`ChannelRegistryEngine{channels: BTreeMap<String, Channel>}`, a `channel-registry` singleton
config doc, backing the chat display layer's All/per-channel/GM views). This sub-project is
strictly the settings-resolution plumbing plus its GM editor UI — not a new subsystem.

## 1. Scope

- Per-channel dice-settings overrides (new work, this spec).
- Per-message overrides: **already fully satisfied by existing inline notation** (`t<N>`, `cs>N`,
  `cf<N>` already force `SuccessCount` mode regardless of ambient settings, per the dice-notation
  skill). No new plumbing — this spec adds one regression test confirming that coverage rather than
  building anything.

## 2. Data model — `DiceSettingsEngine` gains a channel-keyed overrides map

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct DiceSettingsEngine {
    pub mode: DiceModeSetting,
    pub direction: DiceDirectionSetting,
    /// Per-channel full overrides, keyed by `channel-registry`'s channel id. A channel absent
    /// from this map resolves against `mode`/`direction` above (the world default) — this is a
    /// full-replacement override, not a partial merge, matching how the world default itself is
    /// an unconditional pair rather than independently-optional fields.
    #[serde(default)]
    pub channel_overrides: BTreeMap<String, ChannelDiceOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ChannelDiceOverride {
    pub mode: DiceModeSetting,
    pub direction: DiceDirectionSetting,
}
```

Full replacement (not partial-field merge) is deliberate: a partial override ("just change mode,
inherit direction") introduces a second resolution question — does an unset field mean "inherit
world default" or "inherit some other fallback" — that the world-default pair itself never has to
answer, since `mode`/`direction` are always both present with `#[serde(default)]` filling absence
at the whole-document level, not per-field. Keeping the override the same shape as the thing it
overrides avoids inventing a second merge semantics for one small feature.

## 3. Resolution — `resolve_dice_context` gains a channel parameter

```rust
pub async fn resolve_dice_context(repo: &dyn Repository, world: Uuid, channel: &str) -> ParseContext
```

Looks up `channel_overrides.get(channel)`; falls back to `mode`/`direction` on a miss, exactly as
today's absent-doc/malformed-doc fallback already does. `handle_send_message` already has `channel`
available on the incoming request — threading it through is the entire plumbing change on the call
side.

## 4. Scope note on `channel`'s server role — deliberate, not a secrecy change

Today `channel` is documented as "a purely client-chosen label with ZERO server-enforced
meaning." This sub-project gives the server ONE narrow, explicit reason to read it: selecting
which numeric dice-settings pair a roll resolves under. This does not reopen or narrow any
secrecy/authorization boundary — `channel` still does not gate document visibility, message
audience, or any capability check; misresolving it at worst changes which mode/direction a roll
uses, never who can see a message. The "zero server-enforced meaning" claim in the chat-core skill
needs correcting to reflect this one exception (see §6) rather than silently going stale.

## 5. GM authoring UI

`module-game-settings`'s Dice settings section gains a per-channel editor: enumerate
`channel-registry`'s current channel ids (read the same registry the chat display layer already
reads), one row per channel with a mode/direction pair matching the existing world-default
controls' shape, defaulting to "inherit world default" (i.e., no entry in `channel_overrides`)
until the GM explicitly sets a channel's own pair.

## 6. Testing & documentation

- `resolve_dice_context` unit tests: channel with an override resolves to it; channel absent from
  the map falls back to world default; malformed/absent `dice-settings` doc still yields the safe
  `ParseContext::default()` baseline regardless of channel (unchanged existing behavior, now
  re-asserted with a channel argument present).
- One new test confirming the per-message inline-notation override claim in §1 (`t<N>`/`cs>N`
  forces `SuccessCount` regardless of the ambient — and now per-channel — resolved settings).
- `shadowcat-codebase-chat` skill: correct the "zero server-enforced meaning" claim per §4, and
  document the new `channel_overrides` resolution step alongside the existing `resolve_dice_context`
  gotcha.
- `shadowcat-codebase-dice` skill: note `ChannelDiceOverride` as a sibling of `DiceSettingsEngine`
  in the world-settings-doc family.

## 7. Non-goals

- No change to `channel-registry` itself (channel creation/naming/membership) — this consumes it,
  it doesn't extend it.
- No per-message settings UI (already covered by existing notation, §1).
