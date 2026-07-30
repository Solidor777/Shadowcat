# topbar

## Purpose

The top bar: world identity, the launcher menu (opening panels), and live user
presence.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `topbar:topbar` | `shadowcat.surface:topbar` | `TopBar` | — |

## Components

- `TopBar.svelte` — the bar itself.
- `LauncherMenu.svelte` — panel launcher (opens/focuses panels via
  `ctx.panels`).
- `Presence.svelte` — who's connected.

## Contracts & seams

- **Requires** `shadowcat.surface:topbar` (from core-ui). Depends on
  `core-ui ^0.1.0`.
- Reads `ctx.panels` (the panel bridge) for the launcher.

## Pointers

- Source: `src/modules/topbar/`
- API: [`@shadowcat/module-topbar`](/api/ts/modules/_shadowcat_module-topbar.html)
