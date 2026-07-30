# settings

## Purpose

The Settings panel: session controls (locale, leave world, logout) plus the
administration managers — users, world invites, and installed-module
enablement.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `settings:panel` | `shadowcat.panel` | `Settings` | order 6, icon 🔧, labelKey `settings.tab`, launcher-closed |

## Components

- `Settings.svelte` — the panel shell.
- `UserManager.svelte` — admin account management.
- `InviteManager.svelte` — world invite codes.
- `ModuleManager.svelte` — installed community modules: discovery list +
  per-world enable toggles (engine-compat gate surfaced here).

## Contracts & seams

- **Requires** `shadowcat.panel` (from panels); depends on `core-ui ^0.1.0`.
- Talks to the account/invite/module HTTP routes; module enablement drives the
  server's per-world enabled-module set.

## Pointers

- Source: `src/modules/settings/`
- API: [`@shadowcat/module-settings`](/api/ts/modules/_shadowcat_module-settings.html)
