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
- Uses `ctx.searchDocuments` (live FTS), `ctx.openDocument` (sheet panels),
  and `ctx.actorSelection` (hand-off to the place tool).

## Pointers

- Source: `src/modules/actors/`
- API: [`@shadowcat/module-actors`](/api/ts/modules/_shadowcat_module-actors.html)
