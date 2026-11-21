# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Flaky tests

- [Flake, Linux CI, upstream WebKit behaviour] The WebKit leg of `e2e-long-audio`
  (`long-audio-https-webkit`, the `ambient-bed streams over opus-webm` spec) fails intermittently.
  Measured on the Linux runner (GStreamer backend, WebM derivative):
  - The element's `currentTime` runs ahead of the sound it plays. Reading the position code
    through the page's mixed output right after the server `Seek` to 1185 s, the audible code
    changes from 5 to 6 (the 1190 s slot boundary) when the server position is 1191.7 to 1192.1 s
    and the element reports 1192.75 to 1193.13 s, so the element leads the audio by about 2.8 s
    and the audio lags the server by about 1.8 s (Firefox and Chromium: 0.2 to 0.9 s). The same
    lead shows as a constant `currentTime` minus server position of +1.0 s in the spec's samples.
  - `TrackPlayer.sync` trusts `currentTime`, sees a drift over `SYNC_SEEK_THRESHOLD_SECS` every
    tick and hard-seeks about once per second for the whole run (8 of 8 logged runs: a `seeking`
    event every second from 0.4 s to 1199 s). No other engine has a position source the product can
    read instead, so the product cannot correct it.
  - Two failure shapes follow from the seeking. (a) The element fires `ended` about 3.5 s before the
    track's end (element time about 1196.3 s), so the last-slot read at server position 1195.1
    finds it paused (two traces). (b) After the seek near the end WebKit re-requests the whole file
    with no `Range` header instead of the tail range (2 of 8 repeats in one CI run, 0 of 8 in the
    run before), so the spec's "served by a ranged response" check times out.
  Whether the spec accepts WebKit's own early end and its unranged restart is the owner's decision.

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

