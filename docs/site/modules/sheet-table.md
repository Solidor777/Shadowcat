# sheet-table

## Purpose

The sheet for the `table` doc_type, registered at priority 0. Edits the
rollable table's name, description, draw rule (weighted or formula), and
rows (each row's label/weight/range and result entries), and offers a Draw
affordance that posts a chat draw over the world's first channel.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-table:sheet` | `shadowcat.sheet:table` | `TableSheet` | sheet priority 0 |

## Components

- `TableSheet.svelte` — name/description/draw-rule/rows fields; Draw is
  gated by `channel === null` (a missing channel-registry), independently of
  `readOnly` (the write gate).
- `RowEditor.svelte` — one row's label/weight/range and its `results` list.
- `EntryEditor.svelte` — one result entry (`text`/`doc`/`image`/`draw`), with
  a live document/table picker mirroring the composer's `@doc` picker.

## Contracts & seams

- **Provides** `shadowcat.sheet:table` (multi; via `sheetContract(TABLE_DOC_TYPE)`).
- `rowOps.ts` holds pure, `structuredClone`-based helpers over
  `TableEngine.rows`; every mutation replaces the whole array via `setField`
  (`set_pointer` cannot grow arrays).
- The "draw" entry kind's table picker filters `ctx.searchDocuments` hits by
  `doc_type === TABLE_DOC_TYPE` client-side (see the `TODO:` in
  `EntryEditor.svelte` — a server-side `docTypes` search filter is pending).

## Pointers

- Source: `src/modules/sheet-table/`
- API: [`@shadowcat/module-sheet-table`](/api/ts/modules/_shadowcat_module-sheet-table.html)
