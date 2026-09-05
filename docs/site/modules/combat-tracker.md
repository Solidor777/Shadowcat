# combat-tracker

## Purpose

The default combat tracker panel: combats for the viewed scene, ordered
combatant rows, clock controls (start/pause/advance/rewind/sort/end), add/
remove/hide/reorder, per-row and roll-all initiative rolls, resource editing,
and a "your turn" notice badge. Pure presentation over `AppContext.combat`/
`ctx.documents`/`ctx.chat`/`ctx.panels`/`ctx.hooks` — replaceable by any
system or community module contributing its own `shadowcat.panel`.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `combat-tracker:panel` | `shadowcat.panel` | `CombatTrackerPanel` | order 2, icon ⚔️, labelKey `combatTracker.tab`, launcher-closed |

## Components

- `CombatTrackerPanel.svelte` — scene scoping, combat picker, and row
  composition (`CombatHeader` + `CombatantRow` + `AddCombatants`), plus
  pointer-drag and Alt+arrow keyboard reordering.
- `CombatHeader.svelte` — clock controls gated by `CombatAffordances`, the
  two-click End confirm, and "Roll all".
- `CombatantRow.svelte` — one combatant or event row: name/conditions,
  initiative, per-resource cells (tracked stepper, mirror read-only, error
  glyph), and GM-only hide/remove/drag controls.
- `AddCombatants.svelte` — adds the current token selection and authors
  one-off events (name, lifespan, message, hidden).
- `model.ts` — pure helpers: `rowsFor`, `moveInOrder`, `rollTargets`,
  `firstChannel`, `formatResource`.
- `reorder.ts` — the pointer-drag state machine `CombatTrackerPanel` drives.
- `turnBadge.ts` — the launcher badge, lit for the viewer's own turn only.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Reads/writes through `AppContext.combat` (`CombatApi`) — every clock/roll/
  resource action dispatches a server-authorized intent; document-helper
  methods (create/add/remove/setHidden/reorder/setInitiative) write directly.
- Listens to `combat:turn-start`/`combat:turn-end`/`combat:end` hooks to
  drive the launcher badge.

## Pointers

- Source: `src/modules/combat-tracker/`
- API: [`@shadowcat/module-combat-tracker`](/api/ts/modules/_shadowcat_module-combat-tracker.html)
