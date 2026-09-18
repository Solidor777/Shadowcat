# scene-browser

## Purpose

The GM's scene browser: scene list with background thumbnails, create,
configure (deep-links the game-settings per-scene section), **local view** (GM
roam — view any scene without moving players), and **activate** (set the scene
players render).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `scene-browser:panel` | `shadowcat.panel` | `SceneBrowserPanel` | order 6, icon 🗺️, labelKey `sceneBrowser.tab`, **gmOnly**, launcher-closed |

## Components

- `SceneBrowserPanel.svelte` — the list + actions.
- `LevelsEditor.svelte` — a scene's floor authoring UI (toggled per-scene via
  the panel's `levels-toggle` button): add/remove/edit a level's name,
  elevation band (`[bottom, top)`), and background image. Commits the WHOLE
  `SceneEngine.levels` array on any single edit (`structuredClone` + mutate +
  one `/engine/levels` write) — never a per-field patch.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Multi-scene seams on AppContext: `viewedSceneId` (what this client renders),
  `setGmViewedScene` (GM local roam), `sceneSelection` (deep-link into
  game-settings); activation writes `world-settings.activeScene`.
- `LevelsEditor` writes `world-settings.activeScene`'s sibling per-scene field
  `/engine/levels` directly (no separate config document); a level's id is
  server-opaque (`crypto.randomUUID()`, generated client-side at author time,
  never re-derived) and is what `AppContext.viewedLevel`/`LevelSwitcher`/scene-
  tools' elevation-band stamping all key on.

## Pointers

- Source: `src/modules/scene-browser/`
- API: [`@shadowcat/module-scene-browser`](/api/ts/modules/_shadowcat_module-scene-browser.html)
