# topbar

## Purpose

The top bar: world identity, the launcher menu (opening panels), and live user
presence.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `topbar:topbar` | `shadowcat.surface:topbar` | `TopBar` | — |

## Components

- `TopBar.svelte` — the bar itself.
- `LauncherMenu.svelte` — panel launcher (opens/focuses panels via
  `ctx.panels`).
- `Presence.svelte` — who's connected.

## Live player list

The top bar shows one round badge per member of the world. A badge shows the
member's avatar when they have one, otherwise the first letter of their name on
their chosen colour. A small dot on the badge shows whether the member is
connected: green for online, grey for offline. The dot is hidden for a member
whose state is not known yet. Hovering a badge shows the member's name, and the
badge's accessible label adds "online" or "offline".

The list is live:

- On joining a world, the server tells the client who is already online, so a
  player who connects late sees correct dots at once. After a reconnect the
  list is rebuilt from the new snapshot, and a member the snapshot does not list
  shows offline.
- When someone connects or disconnects, their dot changes at once.
- When a member changes their colour, pronouns or avatar in
  **Settings → Profile**, every connected member sees the new colour and avatar
  at once, with no reconnect.

A member who is added to the world while you are connected appears in the list
the next time your client reconnects.

## Contracts & seams

- **Requires** `shadowcat.surface:topbar` (from core-ui). Depends on
  `core-ui ^0.1.0`.
- Reads `ctx.panels` (the panel bridge) for the launcher.
- Reads `ctx.members`, `ctx.presence` and `ctx.memberProfiles` for the player
  list. The shell fills `presence` from each `Welcome`'s `online_users`
  snapshot plus `presence` frames, and `memberProfiles` from the roster read
  plus `member_profile` frames (see the [wire protocol](/protocol)).

## Pointers

- Source: `src/modules/topbar/`
- API: [`@shadowcat/module-topbar`](/api/ts/modules/_shadowcat_module-topbar.html)
