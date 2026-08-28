# Real-Time Per-Recipient Move-Streaming — Design

**Status:** SUPERSEDED by `2026-08-27-move-stream-live-clip-design.md` (user chose direction
2026-08-27 under the new ARCHITECTURE.md invariant 11). Retained as background. Original
status: BLOCKED ON USER INPUT — buddy-checked (two independent reviewers, full debate to
convergence) and the §3 mechanism this document proposed **does not work**. Both reviewers
independently confirmed, then jointly re-confirmed under debate: `Room::execute_move` holds
`publish_guard` across its entire validate→commit body (`ws/room.rs`), fully serializing move
execution, and per-move animation happens ENTIRELY client-side (`TokenAnimator.animateSamples`) —
there is no server-side notion of any token being "mid-flight" at a given wall-clock instant, for
either a bystander token or the recipient's own vision-source token. §3's core operation
("resolve a token's interpolated position at timestamp T") therefore has no target to compute
against under the current architecture, independent of whether an active-stream tracking
structure is built. A real fix requires either (a) new wall-clock-driven interpolation state
tracking every in-flight move's position over real time — complexity comparable to or exceeding
the continuous per-tick loop §2 rejects, not a lighter alternative — or (b) an entirely different
mechanism not yet evaluated, e.g. a targeted vision-correction rebroadcast fired when the
RECIPIENT's own move completes (cheaper, unexplored, may not fully close the gap either). Also
confirmed: the fix as originally scoped is asymmetric (a third token starting mid-animation of an
earlier move is never folded in — the gap relocates rather than closes), and the GM see-as branch
of `clip_move_stream` shares the identical staleness shape and was never addressed.

Given zero correctness/secrecy impact today (confirmed, unchanged) and that a real fix now reads
as its own architecturally deep sub-project — closer in scope to a PLAN.md milestone than a
bucket-C item — this is parked for explicit user direction rather than committing to a costly
redesign unreviewed: **is a full new real-time interpolation subsystem worth building for a
purely cosmetic transient-reveal-timing gap, or is the cheaper "correction on recipient's own
move completion" idea worth designing and evaluating first, or should this stay parked as
low-priority polish?** The rest of this document (§1, §2, §4's questions, §5) remains accurate
background; §3 is superseded by the buddy-check finding above and needs a genuine redesign, not a
patch, once direction is chosen.

**Spec for:** `docs/TODO.md` bucket-C sub-project 8, "Real-time per-recipient move-streaming."

## 1. The gap, precisely

`MoveStream` (M2) computes each move's entire per-recipient vision clip ONCE, at that move's
execute time, against a static vision snapshot. When two tokens move concurrently, recipient R's
clip of token A's stream is computed against R's vision state as it existed when A's move was
processed — R's vision of A does not update mid-walk even if token B's SIMULTANEOUS movement would,
if evaluated live, have opened R's sightline to A sooner. The reveal instead reconciles at A's move
completion + the next `vision` rebroadcast. No secrecy leak either direction — the failure mode is
strictly "reveals later than a truly live system would," never earlier — so this is a visual-fidelity
gap, not a correctness bug.

## 2. Rejected approach: a continuous, always-on per-tick recompute loop

The TODO's own wording ("a per-move server loop recomputing each recipient's visibility of every
concurrently-moving token as positions advance") describes the maximally general fix: a live tick
loop, independent of any single move's sampling, continuously recomputing full visibility for every
recipient against every currently-moving token. This is rejected as the design direction here: it
is a new, always-on, unbounded-frequency computation loop with no natural size bound (world count ×
recipient count × moving-token count, on a wall-clock tick rather than tied to any existing event),
a genuinely different engine primitive from anything else in the vision/movement subsystem, and
disproportionate to a gap with zero correctness impact. It would also duplicate — on a second,
independent clock — exactly the position-sampling machinery `MoveStream` already computes per move,
which is the shape of defect this codebase's own "never fork a decision across two paths" invariant
warns against: two independent computations of "where is this token right now" that must agree.

## 3. Proposed approach: joint clipping across concurrently-active move streams,
reusing each stream's existing sample timeline

When a move command begins execution, the server already knows every OTHER move stream currently
in flight in the same scene (an in-progress `MoveStream` is necessarily tracked somewhere to be
broadcast and to reconcile at completion — the plan locates that existing tracking structure and
reuses it, rather than adding a second one). For a newly-executing move A, instead of clipping A's
samples against a single static vision snapshot:

1. Collect every other move stream B currently active in the same scene.
2. At each of A's existing position samples (no new sampling resolution is introduced — the fix
   widens what a sample is evaluated against, not how many samples exist), resolve B's interpolated
   position at that same timestamp (`B`'s own stream already carries enough samples to interpolate
   from, being itself either static-at-rest or another active `MoveStream`).
3. Compute each recipient's visibility of A's sampled position against the vision-blocking geometry
   AS IT WOULD BE at that instant — which now includes B's current (not pre-move) position wherever
   B itself is vision-relevant (e.g., B is a light source, or B's presence occupies a
   vision-blocking cell) — rather than B's position frozen at A's move-start snapshot.
4. Symmetrically, B's own already-in-flight clip is NOT retroactively recomputed against A (B's
   broadcast already went out) — but B's completion-time reconciliation (the existing "reconciles at
   the stop + next `vision` rebroadcast" step) is unchanged and still closes the gap for any
   watcher who missed the live moment, exactly as today. The fix is therefore: **a newly-starting
   move's clip is computed against the CURRENT positions of already-in-flight concurrent moves,
   instead of their pre-move snapshot** — this closes the specific gap the TODO describes (two
   SIMULTANEOUS moves) without requiring every stream to be continuously re-evaluated against every
   other stream at every instant.

This reuses `MoveStream`'s existing sampling/clipping computation, widening its INPUT (which other
tokens' positions are treated as time-varying rather than static during clip computation) rather
than adding a second, independently-clocked system. It is bounded by "how many moves are
concurrently in flight in one scene" (typically small), not by wall-clock ticks.

## 4. Open questions for the buddy-check (not for the user — these are implementation-risk
questions a second reviewer should pressure-test before a plan is written)

- Whether "vision-relevant" concurrent tokens (step 3) should be ALL other moving tokens or only
  ones whose movement could plausibly affect the recipient's sightline to A (a scoping/performance
  question, not a correctness one — the safe default is "all," narrowed later if profiling shows a
  need).
- Whether the existing in-flight-stream tracking structure this design assumes exists is actually
  suf0ficient to support "list every other active stream in this scene," or needs a small
  extension — this is a codebase-fact question the buddy-check's reviewers should verify against
  the actual `Room`/move-execution code before the plan commits to reusing it.
- Whether recipients who are NOT already watching either A or B need any new work, or whether the
  existing per-recipient broadcast filtering already naturally excludes them (expected: yes, no new
  work — confirm during buddy-check).

## 5. Non-goals

- No continuous per-tick recompute loop (§2).
- No change to how a move's own completion-time reconciliation works — unchanged.
- No change to vision computation itself (`vision::visibility_polygon`) — this only changes WHICH
  positions are fed into the existing per-recipient clipping call, at the existing sample points.
