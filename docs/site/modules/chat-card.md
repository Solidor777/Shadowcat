# chat-card

## Purpose

The default message-card renderer filling chat's singleton `chat.message`
surface. Card chrome (header, roll block, GM recalc menu, edit/delete
affordances) lives here; segment-body rendering — including the client's
**sole `{@html}` boundary** for chat content — is delegated to ui-kit's
`SegmentList`, which every segment kind (text, html, roll embeds, roll
buttons, link previews, oembed cards, doc links, images) renders through.
Replaceable by a game-system module via the same contract.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `chat-card:main` | `shadowcat.surface:chat.message` | `MessageCard` | — |

## Components

- `MessageCard.svelte` — header/roll-block/GM-recalc chrome, edit/delete
  affordances; delegates its segment body to ui-kit's `SegmentList`.

`RollTooltip.svelte` (per-die breakdown tooltip for roll embeds) and
`SegmentList.svelte` (the segment-body renderer and sole `{@html}` sink) now
live in `@shadowcat/ui-kit` — see that package's own docs.

## Contracts & seams

- **Requires** `shadowcat.surface:chat.message` (declared by chat).
- Renders the server-produced message body mirror (`chat-docs.ts`) through
  ui-kit's `SegmentList`; rolls are immutable server artifacts — the card
  renders them, never recomputes them. An `image` segment renders via
  `ctx.assets.url(asset_id, "preview")`, the server's own asset endpoint —
  never a raw external URL.

## Pointers

- Source: `src/modules/chat-card/`
- API: [`@shadowcat/module-chat-card`](/api/ts/modules/_shadowcat_module-chat-card.html)
