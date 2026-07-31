# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

- [Race] Concurrent sessions of the SAME user clobber each other's `ui_state`.
  `PUT /api/me/ui-state` replaces the whole blob (`routes.rs::put_ui_state`),
  and each client session holds its own in-memory snapshot of the entire
  `{global, worlds}` object (`sessionState.svelte.ts`) — a read-modify-write
  with no merge or concurrency control. A session that fetched its snapshot
  before another session's write and persists after it silently reverts the
  other session's slice (e.g. a panel-layout dock made in tab A vanishes when
  tab B persists a locale/lastWorld change). Product impact: same account in
  two tabs/devices. Test impact: the ui-e2e suite runs 6 parallel workers all
  logged in as `ops`, so cross-worker clobbers intermittently break
  `panels.spec.ts` "survives a full page reload" at the panel-restore assert
  (2 of 3 full-suite runs on 2026-07-31; 8/8 green in isolation). Fix
  direction: narrow the write granularity (persist only the changed
  `global`/per-world slice and merge server-side per top-level key) so
  sessions only contend on slices they actually touched.
