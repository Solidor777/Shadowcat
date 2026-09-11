# actors

## Purpose

The actor panel: create, list, search (live FTS), open sheets, and manage
token-visual and ownership details. Also the pick source for the place tool
(what gets stamped onto a scene).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `actors:panel` | `shadowcat.panel` | `ActorsPanel` | order 2, icon 👥, labelKey `actors.tab`, launcher-closed |

## Components

- `ActorsPanel.svelte` — list + live search + create + open sheet.
- `TokenOwnerControl.svelte` — per-actor ownership control.
- `VisualKindEditor.svelte` — token-visual union editing (static/generated
  kinds).
- `FaceSwapPalette.svelte` — face selection for multi-face token visuals.

## Contracts & seams

- **Requires** `shadowcat.panel`; depends on `core-ui ^0.1.0`.
- Uses `ctx.searchDocuments` (live FTS, filtered to actors server-side via
  `docTypes: ["actor"]` — no client-side re-filter), `ctx.openDocument` (sheet
  panels), and `ctx.actorSelection` (hand-off to the place tool).
- The create `<form>` is gated by `ctx.canCreate(ACTOR_DOC_TYPE)` (the
  server's `core:create` mirror) — a restriction the form previously lacked;
  hiding it there hides exactly what the server would refuse. Carried-light,
  hide-name, and ownership controls stay role-gated (`ctx.role === "gm"`),
  since those are Update-path GM affordances, not the Create gate.

## Pointers

- Source: `src/modules/actors/`
- API: [`@shadowcat/module-actors`](/api/ts/modules/_shadowcat_module-actors.html)
