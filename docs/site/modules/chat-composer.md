# chat-composer

## Purpose

The default message composer filling chat's singleton `chat.composer` surface:
an auto-growing textarea that sends via `ctx.chat.send`, with client-side
pre-validation of the cheap rejects. A game-system module may replace it by
claiming the same singleton contract.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `chat-composer:main` | `shadowcat.surface:chat.composer` | `Composer` | — |

## Components

- `Composer.svelte` — input, send-as (actor attribution), audience selection,
  command entry (`/roll ...`).

## Contracts & seams

- **Requires** `shadowcat.surface:chat.composer` (declared by chat).
- Uses `ctx.chat.send` and actor-attribution types (`WireActorOwnerRef`,
  `WireAudience`).

## Pointers

- Source: `src/modules/chat-composer/`
- API: [`@shadowcat/module-chat-composer`](/api/ts/modules/_shadowcat_module-chat-composer.html)
