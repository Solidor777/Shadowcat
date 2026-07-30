# game-settings

## Purpose

The GM's game configuration panel: scene vision/lighting defaults and
per-scene overrides, light gradation, vision modes, pathfinding + movement +
animation settings.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `game-settings:panel` | `shadowcat.panel` | `GameSettingsPanel` | order 5, icon ⚙️, labelKey `gameSettings.tab`, **gmOnly**, launcher-closed |

## Components

- `GameSettingsPanel.svelte` — the whole configuration surface; edits the
  world/vision/lighting config documents through the standard optimistic write
  path.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Reads/writes engine config-docs (world-settings, vision, lighting); the
  scene-browser's "Configure" deep-links into this panel's per-scene section
  via `ctx.sceneSelection`.

## Pointers

- Source: `src/modules/game-settings/`
- API: [`@shadowcat/module-game-settings`](/api/ts/modules/_shadowcat_module-game-settings.html)
