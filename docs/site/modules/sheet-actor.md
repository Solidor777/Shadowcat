# sheet-actor

## Purpose

The generic actor sheet: engine-known fields (display name, faction, shape,
size) as real controls, the opaque `system` band as a tree editor, and the
embedded-items inventory. Registered for the `actor` doc_type at priority 0 —
a game-system module takes over by registering higher (see
[Creating a system](/guides/creating-a-system)).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-actor:sheet` | `shadowcat.sheet:actor` | `ActorSheet` | sheet priority 0 |

## Components

- `ActorSheet.svelte` — the reference implementation of the sheet prop
  contract (`docId` / `systemPrefix` / `close`), the `createSubscriber`
  reactivity bridge, and OCC-correct `setField` writes.

## Contracts & seams

- **Provides** `shadowcat.sheet:actor` (multi).
- Reads the **optimistic** store; `systemPrefix` handles both top-level actors
  (`/system`) and instanced-token embedded copies
  (`/embedded/actor/0/system`); inventory opens embedded items via
  `ctx.openDocument`.

## Pointers

- Source: `src/modules/sheet-actor/`
- API: [`@shadowcat/module-sheet-actor`](/api/ts/modules/_shadowcat_module-sheet-actor.html)
