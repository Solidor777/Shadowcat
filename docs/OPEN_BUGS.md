# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Audio — after an unranged `200` restart, WebKit on Linux plays the element but produces no sound

- [Medium, UNDIAGNOSED — product defect not excluded] In the `long-audio-https-webkit` leg on
  Linux CI, a run ended with the track's `<audio>` element neither paused nor ended at the last
  slot (server position 1194.9 s), yet silent. The earlier reading that the server entry was
  frozen was a misreading: the position read at the end of the 10 s settle with `pausedAt` null.
  The remaining question is whether the element's output reaches Web Audio after the engine
  restarts the stream with a full unranged `200`: either WebKit's GStreamer backend or a
  `MediaElementSource` detaching can produce it, and no measurement separates them. The spec now
  records the element's event log (`play`, `playing`, `seeked`), the `livePausedAt` value per
  read and the traffic start time, so a CI run can answer five open points: `livePausedAt` null
  or not, the event log after `seeked`, network state and `readyState`, event times against
  traffic, and a passing baseline run for comparison. The engine-specific allowance recorded in
  `CLOSED_BUGS.md` covers only the early end and the unranged restart, not silence.

## Flaky tests — two Playwright specs

- [Flake, UNSEPARATED] `levels.spec.ts`'s compound-region authoring press sometimes finds the
  region-create control not committed after a part is added. Three mechanisms are unseparated: no
  active scene at the press (the press does nothing, a user-visible no-op), the compound flag not
  armed at release, and the hit target changing between the actionability check and the press.
  A retry-until-pass edit was rejected because it hides which mechanism fails. `combat-settings.spec.ts`
  has a load-dependent flake of its own that was not reproduced without a run. Both need an
  end-to-end runner that instruments the three mechanisms, loops the combat-settings spec, and
  fixes the product if one is a race.

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

