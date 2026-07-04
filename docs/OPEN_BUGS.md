# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Server / move-execution

- [Movement] `movement::supercover_cells` can spuriously fail-closed (return `None`, rejecting an
  otherwise-legal move) on a diagonal king-step whose leg lands exactly on a 4-way grid-line
  intersection at BOTH endpoints — the Amanatides–Woo corner-crossing branch fires repeatedly and
  drifts the traversal away from the target cell before the `MAX_MOVE_CELLS` guard catches it.
  Reproduced via `execute_move`'s frozen-fixture scenario "diagonal 3-step king path, full
  visible" (`(200,200)→(300,100)` leg): `supercover_cells((200.0,200.0), (300.0,100.0), 100.0)`
  returns `None` even though the move is otherwise fully legal (no wall, no fog, no region).
  Fails closed (never opens a forbidden move) so it is not a security bug, but it rejects a move a
  player would reasonably expect to succeed. Worth a dedicated look by whoever next touches
  `movement.rs`'s corner-crossing branch. (Surfaced by the M10f-2 Task 6 fixture-freeze.)
