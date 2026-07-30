# factions

## Purpose

The world faction registry: seeds three defaults at first GM entry (idempotent)
and provides the GM editor for faction names/colors used by token borders and
actor grouping. Replaceable — a game-system module can supply its own
seed/editor.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `factions:panel` | `shadowcat.panel` | `FactionsPanel` | order 3, icon 🚩, labelKey `factions.tab`, launcher-closed |

## Components

- `FactionsPanel.svelte` — registry editor + group-select affordances.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- The registry lives in the `faction-registry` config-doc's engine band;
  sheets and the render layer read it (actor faction field, token border
  color). Writes `ctx.tokenSelection` for faction group-select.

## Pointers

- Source: `src/modules/factions/`
- API: [`@shadowcat/module-factions`](/api/ts/modules/_shadowcat_module-factions.html)
