# tables

## Purpose

The rollable-table list + quick-draw panel: every readable `table` document,
name-sorted, with a live full-text search, Create, per-row quick Draw, and
Delete.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `tables:panel` | `shadowcat.panel` | `TablesPanel` | order 3, launcher-closed, icon 📋 |

## Contracts & seams

- **Requires** `shadowcat.panel` (`PANEL_CONTRACT`); depends on `core-ui`.
- Delete is gated by `ctx.canDelete(doc)` — never `doc.owner === ctx.selfId`.
- Quick Draw disables while `firstChannel(ctx.documents)` is `null` (the
  pre-resync window); a rejected draw's player-presentable reason
  (`DrawTableError`) surfaces via `ctx.notify`.
- Search sends `ctx.searchDocuments(q, { limit: 20, docTypes: ["table"] })` —
  the one server-side filter, no client-side re-filter.
- Create dispatches `buildTableDoc(ctx.world, name, { draw: { kind:
  "weighted" }, rows: [], description: "" }, { owner: ctx.selfId })` — the
  creator holds `write_fields` (Owner floor) plus `AUTHOR_CAPS`, so a
  granted player edits and deletes the table they made.

## Pointers

- Source: `src/modules/tables/`
- API: [`@shadowcat/module-tables`](/api/ts/modules/_shadowcat_module-tables.html)
