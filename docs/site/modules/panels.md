# panels

## Purpose

The panel manager (`@shadowcat/module-panels`): hosts the dockable panel
surface, renders minimized-panel chips, and provides the `shadowcat.panel`
contract that every panel module (chat, assets, actors, settings, ...)
contributes into. Internally: a pure layout tree with a `LayoutOp` reducer, an
`EngineAdapter` seam (production `DockviewEngine`, test `FakeEngine`), and a
`PanelsController` that owns persisted per-world layout state.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `panels:host` | `shadowcat.surface:panel-host` | `PanelHost` | props: a per-world-session `DockviewEngine` instance |
| `panels:chips` | `shadowcat.surface:panel-dock` | `DockChipsContribution` | — |

## Components

- `PanelHost.svelte` — the docking surface; builds its `PanelsController`
  lazily at mount (register runs in the framework-neutral `ModuleContext`,
  which has no AppContext) and binds it into the shell's shared `PanelsBridge`.
- `DockChipsContribution.svelte` / `DockChips.svelte` — minimized-chip strip,
  reading the same bridge reactively.
- `CompactSwitcher.svelte` — narrow-viewport panel switcher.
- `PanelMenu.svelte` — per-panel menu (float, minimize, pop out).

## Contracts & seams

- **Requires** `shadowcat.surface:panel-host` + `shadowcat.surface:panel-dock`;
  depends on `core-ui ^0.1.0`.
- **Provides** `shadowcat.panel` (multi) — the contract panels contribute into,
  with `PanelMeta` (icon, labelKey, gmOnly, defaultPlacement, badge).
- `ctx.panels` (`PanelsApi` + chips view) is the imperative seam other modules
  use; `ctx.uiState.get/setPanelLayout` persists the layout blob per world.
- The stage well has special veto rules (`STAGE_ID` in `engine/policy`).

## Pointers

- Source: `src/modules/panels/`
- API: [`@shadowcat/module-panels`](/api/ts/modules/_shadowcat_module-panels.html)
