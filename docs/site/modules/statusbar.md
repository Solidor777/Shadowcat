# statusbar

## Purpose

The status bar, plus ownership of the `panel-dock` surface — the strip where
the panel manager renders minimized-panel chips (the hosting module owns the
surface it renders into, matching the core-ui/panel-host precedent).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `statusbar:statusbar` | `shadowcat.surface:statusbar` | `StatusBar` | — |

## Components

- `StatusBar.svelte` — the bar; renders the `panel-dock` surface for the panel
  manager's chip strip.

## Contracts & seams

- **Requires** `shadowcat.surface:statusbar` (from core-ui); depends on
  `core-ui ^0.1.0`.
- **Provides** `shadowcat.surface:panel-dock` (singleton) — filled by the
  panels module's `DockChipsContribution`.

## Pointers

- Source: `src/modules/statusbar/`
- API: [`@shadowcat/module-statusbar`](/api/ts/modules/_shadowcat_module-statusbar.html)
