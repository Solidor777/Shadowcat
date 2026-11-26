# chat-bubbles

## Purpose

A speech bubble appears over the token that just spoke and fades after a few seconds. The bubble
carries the message's own segment list (text, rolls, emotes) rendered through `SegmentList`.
Contributes a DOM overlay into the stage's `STAGE_OVERLAY_CONTRACT` surface. A per-device toggle
(default on) in Settings turns the bubbles off; the chat panel shows every message either way.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `chat-bubbles:overlay` | `shadowcat.stage-overlay` | `BubbleOverlay` | — |
| `chat-bubbles:setting` | `shadowcat.settings-section` | toggle via `registerSetting` | — |

## Behaviour

- Only messages that arrive after the overlay mounts raise a bubble; history never does.
- Message kinds `normal`, `emote` and `roll` raise a bubble; `ooc` and `system` do not, and
  neither does a deleted message.
- The speaker resolves from the message's actor attribution: a `token_instance` owner names its
  token directly; an `actor` owner resolves only when exactly one token linked to that actor is
  placed on the viewed scene and level. An ambiguous or absent speaker raises no bubble.
- A bubble shows for 8 seconds; a token has at most one bubble (a newer message replaces it) and
  at most 8 bubbles are alive at once (the oldest is dropped).
- A bubble is drawn only while the fog the canvas paints reveals the token's position, so a
  bubble never shows where a hidden token stands.
- The bubble is anchored above the token's footprint through `resolveTokenBox` and the stage's
  camera transform (`AppContext.stageView`), so it follows pan and zoom.

## Contracts & seams

- **Provides** `shadowcat.stage-overlay` (multi cardinality); **requires**
  `shadowcat.settings-section` for its toggle.
- Reads `AppContext.stageView` (`StageView`): the stage writes the applied camera transform and
  the fog probe into it; this module reads `sceneToScreen` and `isTokenRevealed`.
- Subscribes directly to `AppContext.documents` for `message` documents; no dependency on
  `chat-card`.
- The toggle is stored per device in `localStorage`, not in the synced UI state.

## Pointers

- Source: `src/modules/chat-bubbles/`
- API: [`@shadowcat/module-chat-bubbles`](/api/ts/modules/_shadowcat_module-chat-bubbles.html)
