# sheet-item

## Purpose

The generic item sheet for the client-only `item` doc_type, registered at
priority 0. Dice-notation string values in the `system` band get a
roll-to-chat affordance.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-item:sheet` | `shadowcat.sheet:item` | `ItemSheet` | sheet priority 0 |

## Components

- `ItemSheet.svelte` — envelope + `system` tree editing; detects dice-notation
  strings and offers roll-to-chat via `ctx.chat`.

## Contracts & seams

- **Provides** `shadowcat.sheet:item` (multi; via `sheetContract(ITEM_DOC_TYPE)`).
- Same sheet prop contract and OCC write discipline as sheet-actor.

## Pointers

- Source: `src/modules/sheet-item/`
- API: [`@shadowcat/module-sheet-item`](/api/ts/modules/_shadowcat_module-sheet-item.html)
