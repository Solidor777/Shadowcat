# sheet-note

## Purpose

The sheet for the `note` doc_type, registered at priority 0. Edits title,
visibility, body (via `SegmentList`), sort order, and the note's tree
position (parent/children), using the draft-base edit flow so a concurrent
remote change surfaces as a conflict rather than being silently overwritten.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `sheet-note:sheet` | `shadowcat.sheet:note` | `NoteSheet` | sheet priority 0 |

## Components

- `NoteSheet.svelte` — title/visibility/sort fields, a draft-base body editor
  over `SegmentList`, and a children list (sorted by `engine.sort` then
  `created_at`) with a `ctx.canCreate`-gated "new child note" affordance.

## Contracts & seams

- **Provides** `shadowcat.sheet:note` (multi; via `sheetContract(NOTE_DOC_TYPE)`).
- The visibility field is gated by `ctx.canEdit(doc, "/permissions/default")`,
  distinct from the body's own write gate.
- Draft-base flow: opening the sheet seeds both a live `draft` and an
  immutable `draftBase` snapshot; Save writes with `draftBase` (not the live
  stored value) as the OCC `old`, so a concurrent remote edit rejects with
  `conflict` instead of overwriting it.

## Pointers

- Source: `src/modules/sheet-note/`
- API: [`@shadowcat/module-sheet-note`](/api/ts/modules/_shadowcat_module-sheet-note.html)
