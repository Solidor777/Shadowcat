# settings

## Purpose

The Settings panel: session controls (locale, leave world, logout) plus the
administration managers — users, world invites, and installed-module
enablement.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `settings:panel` | `shadowcat.panel` | `Settings` | order 6, icon 🔧, labelKey `settings.tab`, launcher-closed |
| `settings:keybindings` | `shadowcat.settings-section` | `KeybindingsSection` | labelKey `settings.keybindings.title` |
| `settings:backups` | `shadowcat.settings-section` | `BackupManager` | labelKey `settings.backups.title`, `serverAdminOnly` |

## Components

- `Settings.svelte` — the panel shell.
- `UserManager.svelte` — admin account management.
- `BackupManager.svelte` — server-admin backup management, mounted as a
  settings section (a section with `serverAdminOnly` is rendered, heading
  included, only for a server admin). See below.
- `InviteManager.svelte` — world invite codes.
- `ModuleManager.svelte` — installed community modules: discovery list +
  per-world enable toggles (engine-compat gate surfaced here), plus a second
  "Run sandboxed validators" toggle for any enabled module that declares
  `validators` in its manifest (the per-world `validators_enabled` opt-in,
  persisted as `WorldModuleEntry`). A module whose declared validator fails to
  compile shows its load error inline beside the toggle
  (`InstalledModuleInfo.validator_load_error`, from the server's registry
  scan). See [Creating a validator](/guides/creating-a-validator).
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

## Backups

`BackupManager.svelte` lists every backup under the server's backups folder,
newest first: when it was taken, why (manual, before a restore, scheduled),
size, asset-file count and the server version that wrote it.

- **Back up now** takes a manual backup behind an indeterminate progress
  notification, then reloads the list. It is the one home of the button.
- **Restore** asks for confirmation first. The server saves a safety backup of
  its current state, stages the restore and stops; it does not restart itself.
  Start it again to apply the restore (see the [hosting guide](/guides/hosting)).
  If a staged restore could not be applied, a banner on the list names the
  backup and the error, and the server runs on the data it had.
- **Delete** moves a backup into the server trash after a confirm dialog.
- The **retention policy** is read-only here: it is set in the server's
  configuration (`backup_keep_last`, `backup_keep_days`). **Apply now** runs it
  immediately; it is disabled when no limit is set.

Scheduled backups are not built; the manager only lists and labels them.

## Contracts & seams

- **Requires** `shadowcat.panel` (from panels); depends on `core-ui ^0.1.0`.
- **Provides** `shadowcat.settings-section`; `SettingsSectionMeta.serverAdminOnly`
  hides a section and its heading from everyone but a server admin (advisory:
  the section's own routes are admin-gated server-side).
- Talks to the account/invite/module/backup HTTP routes; module enablement
  drives the server's per-world enabled-module set.

## Pointers

- Source: `src/modules/settings/`
- API: [`@shadowcat/module-settings`](/api/ts/modules/_shadowcat_module-settings.html)
