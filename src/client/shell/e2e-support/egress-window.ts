/** The server `egress_window_secs` (seconds) the media-range and long-audio test servers run with.
 * Long enough for one browser read-ahead burst to finish, short enough that a response left idle
 * across a real pause between bursts is cut inside a test's budget. The Playwright configuration
 * sets it on both servers and the long-audio play-through derives its gap threshold from it, so the
 * server and the assertion cannot drift apart. */
export const MEDIA_RANGE_EGRESS_WINDOW_SECS = 3;
