# chat

## Purpose

The chat panel host: channel tabs, the message stream, and the unread badge.
The default panel — order 0, docked right — and the declaration point for the
two singleton surfaces that composer/card modules fill. Renders gracefully with
neither filled, so it lands independently of them.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `chat:panel` | `shadowcat.panel` | `ChatPanel` | order 0, icon 💬, labelKey `chat.tab`, defaultPlacement docked-right, live unread badge |

## Components

- `ChatPanel.svelte` — channels, stream, read markers; renders the
  `chat.composer` and `chat.message` surfaces.

## Contracts & seams

- **Requires** `shadowcat.panel`.
- **Provides** `shadowcat.surface:chat.composer` (singleton) and
  `shadowcat.surface:chat.message` (singleton).
- Uses `ctx.chat` (send/edit/delete), `ctx.uiState.get/setChatRead` (unread
  markers), and the panel badge seam (`PanelBadge`) for the live unread count.

## Pointers

- Source: `src/modules/chat/`
- API: [`@shadowcat/module-chat`](/api/ts/modules/_shadowcat_module-chat.html)
