# game-settings

## Purpose

The GM's game configuration panel: scene vision/lighting defaults and
per-scene overrides, light gradation, vision modes, pathfinding + movement +
animation settings, the combat rules chain (world tier + per-scene
overrides, with provenance hints and an effective-rules summary), and the
resource registry editor.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `game-settings:panel` | `shadowcat.panel` | `GameSettingsPanel` | order 5, icon ⚙️, labelKey `gameSettings.tab`, **gmOnly**, launcher-closed |

## Components

- `GameSettingsPanel.svelte` — the whole configuration surface; edits the
  world/vision/lighting config documents through the standard optimistic write
  path.
- `CombatSettings.svelte` — the world-tier combat rules chain editor
  (movement resource, budget interpretation, enforcement, turn control,
  effect cleanup/rewind/forward restore, effect-lifecycle formulas), each
  leaf with a provenance hint and reset, plus an effective-rules summary for
  the selected scene.
- `CombatSceneOverrides.svelte` — the same eight controls scoped to the
  selected scene's `/engine/combat`, where Inherit falls through to the
  world tier.
- `ResourceRegistryEditor.svelte` — the GM resource-registry editor: add/
  remove entries, edit name/order/kind, and per-kind formula fields (Mirror
  value; Tracked max + four recovery boundaries).

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Reads/writes engine config-docs (world-settings, vision, lighting); the
  scene-browser's "Configure" deep-links into this panel's per-scene section
  via `ctx.sceneSelection`.

## Pointers

- Source: `src/modules/game-settings/`
- API: [`@shadowcat/module-game-settings`](/api/ts/modules/_shadowcat_module-game-settings.html)
