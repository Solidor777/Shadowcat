# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Audio — a streaming track's element drifts ahead of the server position at about 2 percent per second

- [High] `TrackPlayer.sync`'s streaming arm sets `el.playbackRate` to `1 + SYNC_RATE_NUDGE` when the
  element is behind the server position by at most `SYNC_SEEK_THRESHOLD_SECS` and returns it to 1
  only when the drift is exactly 0, and `sync` runs only when `AudioEngine.applyState` does (on an
  `audio-state` document change; no periodic call exists). The element is always slightly behind at
  the first call (it has not started playing), so the nudge engages once and stays: measured over a
  20-minute single-track play-through (plain HTTP and HTTPS, Chromium) the element's `currentTime`
  minus the server position was +0.08, +0.29, +0.50, +0.68, +0.90, +1.09 seconds at server positions
  5, 15, 25, 35, 45, 55 seconds (0.0202 per second, the nudge size), and +6.05 seconds at 305 seconds,
  where the audible position code left the server position's 10-second slot. A server `Seek`
  re-syncs the element (drift -0.01 afterwards), which is why a short sampled run does not show it.
  The long-audio play-through e2e (`long-audio-playthrough.spec.ts`) is red from this defect alone.
  The fix is the synchronisation tick with a rate deadband the audio item of the current plan
  builds; this entry closes with it.

## Upstream — dockview-core `onDidPopoutGroupPositionChange` populates `screenY` from `window.screenX`

- [Low, UPSTREAM — routed around] In the vendored `dockview-core` popout wiring
  (`DockviewComponent`'s `onDidWindowMoveEnd` listener inside `addPopoutGroup`), the
  `_onDidPopoutGroupPositionChange` emitter fires with `screenX: window.screenX,
  screenY: window.screenX` — the payload's `screenY` is a copy of `screenX`, never the
  window's real vertical position. Any consumer reading the event payload persists a
  corrupt Y coordinate. Shadowcat never consumes the payload:
  `DockviewEngine.#handlePopoutGeometryChange` reads geometry from the popout entry's
  own window (`DockviewApi.getPopouts()` match on the group →
  `screenX`/`screenY`/`innerWidth`/`innerHeight`, the same four fields
  `PopoutWindow.dimensions()` reads). If the upstream defect is fixed, no Shadowcat
  change is needed; if the event payload is ever consumed here, re-verify against the
  vendored source first.

