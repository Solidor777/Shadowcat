# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## [Client] Every per-scene vision/lighting override is broken on a default-created scene
`buildSceneDoc` writes `vision: null` and `lighting: null`, and `setPointer`
(`src/client/core/src/store.ts:50`) auto-creates a MISSING intermediate key but cannot descend
through an explicit `null`. So dispatching `/engine/vision/movementRestriction` — or any of the ten
scene-tier vision/lighting overrides in `GameSettingsPanel` — throws `cannot set field on
non-container` as an uncaught page error, and the control silently does nothing. Repro: create a
scene through the normal UI (the scene browser's create button), open game settings, change any
per-scene vision or lighting override. The world-tier controls are unaffected and scenes inherit
from them, which is why this has gone unnoticed — the product path works, the override path does
not. Fix direction is a design call: either `buildSceneDoc` should omit the keys rather than write
`null` (so `setPointer`'s auto-create applies), or `setPointer` should treat an explicit `null`
intermediate as replaceable. Prefer whichever keeps "absent" and "explicitly null" meaningfully
distinct elsewhere in the document model. (Found by the Task 14c player e2e.)

## [Client, more serious] A failed optimistic op appears to be replayed and wedges the client
After the failure above, the NEXT unrelated intent (the snap toggle) threw the SAME
`cannot set field on non-container` message and never committed — i.e. the failed op is apparently
retained and re-applied ahead of subsequent intents, so one bad dispatch blocks all later ones on
that client until reload. This is independent of the `null`-intermediate cause above: any op that
throws inside the optimistic apply path would presumably wedge the queue the same way. That makes
it the more dangerous of the two — the first is a broken control, this is a client that stops
accepting writes with no user-visible explanation. Needs its own investigation: establish whether
the failed op is genuinely re-applied or whether the queue is left in a state that re-throws, and
add a regression test that dispatches a known-bad op followed by a good one and asserts the good
one commits. (Found by the Task 14c player e2e.)
