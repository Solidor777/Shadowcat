# onboarding

## Purpose

First-run guided tours. Any module contributes a tour as plain data; this module
is the engine that offers the eligible one, draws it as a spotlight plus a step
card, remembers which tours each account finished, and lists every tour in a
"Replay tours" settings section. A tour is always optional: the offer is a
dismissible toast, never a modal, and the user can skip a running tour, silence
offers for good, or replay a tour later.

To ship a tour from your own module, see
[Contributing a guided tour](/guides/creating-a-module#contributing-a-guided-tour).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `onboarding:host` | `shadowcat.surface:overlay` | `OnboardingHost` | (none — draws the offer toast and the tour overlay) |
| `onboarding:replay` | `shadowcat.settings-section` | `ReplaySection` | labelKey `onboarding.replay.title` |

The module **provides** `shadowcat.tour` (multi), the contract other modules
contribute their tours under. It **requires** `shadowcat.settings-section` and
`shadowcat.surface:overlay`.

## Shipped tours

Seven first-party tours ship, each declared by the module that owns the screen it
points at. Every tour is version 1, audience `any`, scope `global`.

| Tour id | Module | Trigger | Steps (anchor ids) |
|---|---|---|---|
| `stage.intro` | `stage` | first use | `stage.canvas`, `stage.gm-view` (the GM's vision picker; skipped for a player) |
| `scene-tools.intro` | `scene-tools` | first use | `scene-tools.rail`, then the `select`, `place`, `measure` and `ping` tool buttons (`scene-tools.select` and so on; the place button is GM-only, so a player skips it) |
| `chat.intro` | `chat` | first use | `chat.panel`, `chat.channels`, `chat.messages` |
| `chat-composer.dice` | `chat-composer` | first use | `chat-composer.input`, `chat-composer.roll-mode`, `chat-composer.doc-link` |
| `actors.intro` | `actors` | **replay only** | `actors.panel`, `actors.search`, `actors.list`, `actors.quick-edit` (skipped until exactly one token is selected) |
| `notes.intro` | `notes` | **replay only** | `notes.panel`, `notes.search` |
| `settings.intro` | `settings` | **replay only** | `settings.panel`, `settings.language` |

The actors, notes and settings panels start closed, so their anchors are not on
screen at first run and an automatic offer would have nothing to point at. Those
three tours run only from **Replay tours**, with the panel open.

## When a tour is offered

The offer is decided once the UI has mounted, and again whenever a module
contributes or removes a tour. A tour is offered when all of these hold:

- Its trigger is `first-use`.
- Its audience includes the viewer: `gm` and `player` match that world role
  exactly, `admin` matches a server admin, `any` matches everyone.
- The viewer has not finished its current version.
- At least one of its steps has a mounted anchor.
- The user has not chosen "Don't ask again", and the server has not switched
  tours off (see below).

At most **one** tour is offered per session, the first eligible in module
registration order. Tours not offered wait for a later session or the replay
list. The toast has three buttons:

| Button | Effect |
|---|---|
| Start | Runs the tour. |
| Not now | Closes the toast for this session and records nothing, so the tour is offered again next session. |
| Don't ask again | Stops all automatic offers for this account. Replay still works. |

A step whose anchor is not mounted is skipped, and the "Step n of m" counter
counts only the steps that can be shown. Skip, Done and Escape all end the tour
and record its current `version` as finished; raising a tour's `version` offers
it again to everyone who finished an older one.

## Replay

**Settings → Replay tours** lists every tour whose audience includes the viewer,
with a Replay button on each. Replay starts the tour at its first step that can
be shown. If no step of the tour is on screen (for example, the tour is for a
panel that is closed), it shows "This tour has nothing to show on the current
screen." Replay is not blocked by "Don't ask again" or by the server switch.

## Progress storage

Done-state lives in the account's UI state, under the module's own slice
(`ui_state.modules.onboarding`), written through `ModuleContext.settings.setUser`.
It follows the account across devices and worlds. Keys are flat, one per tour:

| Key | Value |
|---|---|
| `g:<tour id>` | The finished version, for a `global`-scoped tour. |
| `w:<world id>:<tour id>` | The finished version, for a `world`-scoped tour. |
| `disabled` | `true` once the user chose "Don't ask again". |

One key per tour means two sessions of one account finishing different tours do
not overwrite each other. The module keeps this session's own writes in memory
so a tour finished a moment ago is never offered again while its write is in
flight.

## Server switch

The server config boolean `onboarding_enabled` (default `true`; see the
[hosting guide](/guides/hosting)) is published in `GET /api/config`. Only an
explicit `false` suppresses automatic offers: a failed request or a body that
does not parse counts as "allowed". The route needs no sign-in. The switch
affects automatic offers only; Replay is unaffected.

## Entry welcome screen

The [entry](/modules/entry) package shows a one-screen welcome after the first
administrator account is created: "Your server is ready", with a link to the
hosting guide and a Continue button leading to login. It is skipped when
`onboarding_enabled` is `false`. The link points at the guide in the project's
online repository, because no world (and so no served guide) exists at that point.

## Touch, keyboard and motion

- The step card is a bottom sheet on a compact viewport, with 44 px buttons; a
  downward touch drag on the sheet ends the tour.
- Escape ends the tour, through the shared Escape coordinator at modal priority.
- Focus is trapped inside the card, moves to the Next button on each step, and
  returns to the previously focused element when the tour ends.
- The spotlight animates only when the user has no `prefers-reduced-motion`
  preference.
- The overlay is plain DOM and uses no secure-context-only browser API, so it
  works on a plain-HTTP LAN origin.

## Components

- `OnboardingHost.svelte` — attaches the controller to the running app and
  evaluates the offer; renders the toast and the overlay.
- `TourOffer.svelte` — the offer toast.
- `TourOverlay.svelte` — the spotlight, step card, Next/Back/Skip.
- `ReplaySection.svelte` — the "Replay tours" settings section.
- `controller.svelte.ts` (`TourController`) — offer, start, step, end.
- `eligibility.ts` (`offerableTours`, `resolvableSteps`, `matchesAudience`) —
  who is offered what.
- `progress.ts` (`TourProgress`, `doneKey`) — done-state.
- `serverFlag.ts` (`fetchOnboardingEnabled`) — the server switch.
- `placement.ts` (`placeCard`) — card placement beside the target.

## Contracts & seams

- Tours are read through `ContributionRegistry.contributionsFor(TOUR_CONTRACT)`.
  Each contribution has `component: null` and a `tour` (`TourMeta`), validated
  when contributed.
- A step's `target` is a `data-tour` anchor id (`TOUR_ANCHOR_ATTR`), looked up
  with `findTourAnchor` from `@shadowcat/ui-kit`. It is never a CSS selector.
- A `<Surface>` host wraps a contribution that sets `Contribution.tourAnchor` in
  a `display: contents` element carrying the attribute.
- Removing the module removes every offer and replay entry; contributed tours
  stay registered and inert.

## Pointers

- Source: `src/modules/onboarding/`; tour contract in `src/client/core/src/tour.ts`
- Guide: [Contributing a guided tour](/guides/creating-a-module#contributing-a-guided-tour)
- API: [`@shadowcat/module-onboarding`](/api/ts/modules/_shadowcat_module-onboarding.html)
