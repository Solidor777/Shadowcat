# conditions

## Purpose

The world condition registry: seeds a generic emoji condition set at first GM
entry (idempotent), provides the GM editor, and a selection-driven palette for
toggling conditions on selected tokens. Replaceable by a game-system module.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `conditions:panel` | `shadowcat.panel` | `ConditionsPanel` | order 4, icon ✨, labelKey `conditions.tab`, launcher-closed |

## Components

- `ConditionsPanel.svelte` — registry editor + toggle palette for the current
  token selection.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- The registry lives in the `condition-registry` config-doc's engine band;
  token condition markers render from it. Reads `ctx.tokenSelection`.

## Pointers

- Source: `src/modules/conditions/`
- API: [`@shadowcat/module-conditions`](/api/ts/modules/_shadowcat_module-conditions.html)
