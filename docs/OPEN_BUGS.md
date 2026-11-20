# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Flaky tests

- [Flake, Linux CI] `egress_stall`'s `an_http1_peer_that_keeps_reading_is_not_cut_across_three_windows`
  fails intermittently on the Linux CI leg: across four consecutive Linux runs it failed in the first
  and third (the cut came at 2.94 s in the third) and passed in the second and fourth. It has not failed on Windows (72 of 72 local
  passes with 24 concurrent copies). Measured on Windows: the socket accepts constant 262 272 B lumps
  roughly every 220 to 330 ms at 64 floors, unchanged with a 64 KiB receive buffer. The test now
  prints the bytes read and the accepted grants (time, bytes) when it fails, so the next Linux
  failure carries the Linux lump sizes. Root cause not proven; no Linux lump data exists yet.

- [Flake, Linux CI] The WebKit leg of `e2e-long-audio` (`long-audio-https-webkit`, the
  `ambient-bed streams over opus-webm` spec) fails intermittently: it failed in one CI run (the
  element was paused at server position 1195.104 where the spec expects it playing, near the end of
  the track) and passed in the next two. (An earlier run also failed this leg, with a Firefox leg
  failing beside it.) Root cause not proven.

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

