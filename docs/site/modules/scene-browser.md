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

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Multi-scene seams on AppContext: `viewedSceneId` (what this client renders),
  `setGmViewedScene` (GM local roam), `sceneSelection` (deep-link into
  game-settings); activation writes `world-settings.activeScene`.

## Pointers

- Source: `src/modules/scene-browser/`
- API: [`@shadowcat/module-scene-browser`](/api/ts/modules/_shadowcat_module-scene-browser.html)
