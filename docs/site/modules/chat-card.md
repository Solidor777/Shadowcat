# chat-card

## Purpose

The default message-card renderer filling chat's singleton `chat.message`
surface. Contains the client's **sole `{@html}` boundary** for chat content:
the fail-closed body parse and sanitized-HTML rendering of message segments,
including roll embeds and link previews. Replaceable by a game-system module
via the same contract.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `chat-card:main` | `shadowcat.surface:chat.message` | `MessageCard` | — |

## Components

- `MessageCard.svelte` — segment rendering (text, roll embeds, roll buttons,
  link previews, system notices), edit/delete affordances.
- `RollTooltip.svelte` — per-die breakdown tooltip for roll embeds.

## Contracts & seams

- **Requires** `shadowcat.surface:chat.message` (declared by chat).
- Renders the server-produced message body mirror (`chat-docs.ts`); rolls are
  immutable server artifacts — the card renders them, never recomputes them.

## Pointers

- Source: `src/modules/chat-card/`
- API: [`@shadowcat/module-chat-card`](/api/ts/modules/_shadowcat_module-chat-card.html)
