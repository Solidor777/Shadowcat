# entry

## Purpose

The pre-world entry experience: login, first-run setup, and world selection.
The shell renders it for any pre-world route. A self-hoster can replace this
package to integrate external auth/identity without touching the in-world UI.

## Contributions

None — `entry` is not a contract-registered module; it exports the `Entry`
component directly, and the shell mounts it for pre-world routes. It is the one
"module" that runs before a world session (and therefore before the module
registry) exists.

## Components

- `Entry.svelte` — the combined login / setup / welcome / world-select flow.
- `views/Welcome.svelte` — the one-screen welcome described below.

## Welcome screen

After the first administrator account is created on a new server, the flow shows
a welcome screen before login: the heading "Your server is ready", a line telling
the admin to sign in with the account just made, then create a world and invite
players, a "Read the hosting guide" link, and a Continue button that leads to the
login form. The link opens the hosting guide in the project's online repository in
a new tab, because no world, and so no served guide, exists yet.

The screen shows only straight after setup, and only while the server's
`onboarding_enabled` setting is on (the default; see the
[hosting guide](/guides/hosting)). With the setting off, setup goes straight to
login. The value comes from `GET /api/config`. See
[onboarding](/modules/onboarding) for the guided tours that follow in the world.

## Contracts & seams

- Consumed by the shell's router directly (no `provides`/`requires`).

## Pointers

- Source: `src/modules/entry/`
- API: [`@shadowcat/module-entry`](/api/ts/modules/_shadowcat_module-entry.html)
