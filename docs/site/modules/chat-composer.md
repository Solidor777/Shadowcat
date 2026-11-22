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
  command entry (`/roll ...`), document-link insertion (`@doc`), and image
  insertion via the asset picker (`ctx.pickAsset({ kind: "image" })`, hidden
  when the world's `chat-settings.images` toggle is off).

## Contracts & seams

- **Requires** `shadowcat.surface:chat.composer` (declared by chat).
- Uses `ctx.chat.send` and actor-attribution types (`WireActorOwnerRef`,
  `WireAudience`).
- Inserts a `[[asset:<id>|<label>]]` span at the cursor on a resolved pick
  (`label` is always the id's first 8 characters — no asset-name lookup
  surface exists on `AppContext`); a cancelled pick is a no-op.

## Roll flavor and link labels

Text typed after the first `|` of a roll command is the roll's flavor, a plain caption shown
beside the result and searchable: `/roll 1d20+5 | Attack roll`. A roll button takes the same
form with a label and a flavor: `[[roll:1d20|Attack|Swing the axe]]`; the click re-sends the
flavor. A document or token link may omit its label (`[[doc:<id>]]`, `[[token:<id>]]`); the
card then shows the target's current name, or "Unknown document" / "Unknown token" when the
viewer cannot see it. An explicit label (`[[doc:<id>|Label]]`) is kept as written.

## Pointers

- Source: `src/modules/chat-composer/`
- API: [`@shadowcat/module-chat-composer`](/api/ts/modules/_shadowcat_module-chat-composer.html)
