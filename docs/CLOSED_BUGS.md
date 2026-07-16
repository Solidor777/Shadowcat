# Closed Bugs

Resolved defects, kept for reference. Move an entry here from `OPEN_BUGS.md` once its fix has
landed; do not use this file as a to-do list.

## Server / data (OCC)

- [Critical, FIXED] `apply_intent`'s Phase-1 OCC pre-image comparison (`data/sqlite.rs`,
  `actual != ch.old`) used raw `serde_json::Value` equality, which spuriously rejected an
  otherwise up-to-date write. Mechanism: `serde_json::Value::Number` splits whole numbers into
  `PosInt`/`NegInt` and non-whole numbers into `Float`; the server stores an M13-0 `engine` `f64`
  field as `Float` even when its value is a whole number (e.g. `100.0`), but a JS client cannot
  preserve "this was a float" for a whole-number value through `JSON.parse`/re-serialize — the
  echoed OCC pre-image comes back as `PosInt`, and raw `==` treats `PosInt(100)` and `Float(100.0)`
  as unequal. Reachable via an ordinary token drag (`sendMoves`,
  `src/modules/scene-tools/src/controller.svelte.ts`) performed any time after a server-executed
  move (`execute_move`, which commits `/engine/x,y` as `Float`), and via the `ActorsPanel`
  vision-range editor and `GameSettingsPanel` numeric editors, whose pre-images are nested
  arrays/objects containing whole-number `Float` leaves. Fix: `values_semantically_eq`
  (`data/sqlite.rs`), a structural equality that recurses into `Object`/`Array` and treats
  mismatched-variant `Number` leaves as equal when numerically equal and the integer side is
  exactly representable as `f64` (`|n| <= 2^53`), falling back to exact comparison otherwise.
  Scoped to the OCC pre-image comparison only — Phase-2 normalization and all other equality
  checks are untouched. Regression coverage: 5 unit tests on `values_semantically_eq` (whole-number
  Float/PosInt equality, genuinely stale rejection, nested array/object recursion, >2^53 precision
  fallback, negative-number variant mismatch) plus an integration test
  (`ws::room::room_tests::client_update_with_posint_pre_image_after_execute_move_is_accepted`)
  reproducing the real `execute_move` → client-drag path end to end.
