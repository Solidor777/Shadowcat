/** The shared-ruler limits the server enforces on `ruler_update` frames, mirrored for the client
 * sender. A shared conformance fixture (`__fixtures__/ruler-limits.json`) is read by this
 * package's test and by the server's `ws::conn::tests::ruler` test (`include_str!`), so a change to either
 * side's number fails the other side's test instead of drifting. */
export const RULER_LIMITS = {
  /** The most vertices one `ruler_update` may carry (`ws::ruler::MAX_RULER_POINTS`); a longer
   * list is answered with a generic `error` frame to the sender and never relayed. */
  maxPoints: 256,
  /** The longest `origin` token, in bytes (`ws::ruler::MAX_RULER_ORIGIN_BYTES`); a frame with a
   * longer one is dropped silently. */
  maxOriginBytes: 64,
  /** The per-user, per-trailing-minute frame budget (`ws::ruler::RULER_RATE_PER_MIN`); frames
   * past it are dropped silently. */
  ratePerMin: 720,
  /** The largest coordinate magnitude, in scene units, of any vertex
   * (`scene::move_exec::MAX_GATE_WALK_COORD`); a frame with a finite coordinate beyond it is
   * dropped silently and a non-finite one is answered with a generic `error` frame. */
  maxCoord: 1e9,
} as const;
