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
  mismatched-variant `Number` leaves as equal when numerically equal. Same-variant integer PAIRS
  (both PosInt/NegInt) are compared EXACTLY as `i128` with no magnitude limit; the `|n| <= 2^53`
  exactness guard applies only to the mixed case (one integer side, one `Float` side), where an
  `f64` comparison is unavoidable. Scoped to the OCC pre-image comparison only — Phase-2
  normalization and all other equality checks are untouched. Regression coverage: 9 unit tests on
  `values_semantically_eq` (whole-number Float/PosInt equality, genuinely stale rejection, nested
  array/object recursion, >2^53 mixed-case precision fallback, negative-number variant mismatch,
  large same-variant integer pairs that alias under f64 but must reject, opposite-sign
  same-magnitude rejection, trivially-equal small integers) plus an integration test
  (`ws::room::room_tests::client_update_with_posint_pre_image_after_execute_move_is_accepted`)
  reproducing the real `execute_move` → client-drag path end to end.
- [Critical, FIXED] Round 2: the fix above's Number-comparison branch had no magnitude guard when
  BOTH sides parsed as same-variant integers, falling through to the lossy `f64` equality used for
  the mixed case. Two distinct same-variant integers whose magnitude exceeds 2^53 (e.g. `2^62` vs
  `2^62 + 1`) alias to the same `f64` and were incorrectly reported equal — an OCC bypass in the
  silent-lost-update direction, strictly worse than raw equality for this case (raw equality would
  have correctly rejected them). Fix: the both-integers case now compares as `i128` exactly and
  never falls through to `f64`; the `f64`-tolerant path is reserved exclusively for the genuinely
  mixed integer/`Float` case. Regression coverage: 4 additional unit tests (large same-variant
  PosInt pair that aliases under `f64`, large same-variant NegInt pair, opposite-sign
  same-magnitude rejection, trivially-equal small-integer pair).
