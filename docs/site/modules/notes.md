# notes

## Purpose

The note tree + search panel: every readable `note` document rendered as a
parent/children tree (a child whose parent is unreadable is promoted to
root), a live full-text search that replaces the tree with a flat hit list,
Create, per-row Delete/Move-to, and expand/collapse.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `notes:panel` | `shadowcat.panel` | `NotesPanel` | order 3, launcher-closed, icon 📓 |

## Components

- `NotesPanel.svelte` — tree/search state, the create form (`ctx.canCreate`-gated).
- `NoteTree.svelte` — the recursive row renderer; also renders a search hit
  list (each hit as a childless node), so a row's Delete/Move-to/open
  affordances never fork into a second markup shape.
- `tree.ts` — `buildNoteTree`, the pure root-promotion + `(engine.sort,
  created_at)` ordering helper.

## Contracts & seams

- **Requires** `shadowcat.panel` (`PANEL_CONTRACT`); depends on `core-ui`.
- Delete is gated by `ctx.canDelete(doc)` — never `doc.owner === ctx.selfId`.
- Move-to is GM-only (`Operation::Move` is GM-only server-side) and
  dispatches `buildMoveOp(doc.id, target, doc.parent_id ?? null)` from
  `@shadowcat/core` — the same builder the asset folder tree uses.
- Search sends `ctx.searchDocuments(q, { limit: 20, docTypes: ["note"] })` —
  the one server-side filter, no client-side re-filter.
- Create dispatches `buildNoteDoc(ctx.world, name, "", { owner: ctx.selfId })`
  — private by default; the sheet's visibility control shares it.

## Pointers

- Source: `src/modules/notes/`
- API: [`@shadowcat/module-notes`](/api/ts/modules/_shadowcat_module-notes.html)
