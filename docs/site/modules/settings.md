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
- `PerformanceEditor.svelte` — the built-in per-device render-budget section
  (see below).

## Performance

`PerformanceEditor.svelte` is a built-in section of the Settings panel (not a
contribution — the theme editor is the precedent), inserted between the theme
block and the module manager. It drives `AppContext.performance` (a
`PerformanceController`, per-device only, persisted in `localStorage` — never
the server `ui_state`). Its controls: a preset radio (`auto` / `mobile` /
`balanced` / `quality`, with a `custom` badge once any field diverges), a
frame-rate-cap select, a render-scale range (0.5–1), a lighting-quality
select, boolean toggles for antialiasing, token effects, visual effects, 3D
dice, spatial audio, idle redraw-skipping and reduced motion, a "Show frame
stats" toggle (surfaced by the statusbar's `PerfStats`), and a "Reset to auto"
button. See the [Performance guide](/guides/performance) for what each knob
costs.

## Contracts & seams

- **Requires** `shadowcat.panel` (from panels); depends on `core-ui ^0.1.0`.
- Talks to the account/invite/module HTTP routes; module enablement drives the
  server's per-world enabled-module set.

## Pointers

- Source: `src/modules/settings/`
- API: [`@shadowcat/module-settings`](/api/ts/modules/_shadowcat_module-settings.html)
