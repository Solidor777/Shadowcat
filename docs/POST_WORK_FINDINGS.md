# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: `slow_reader_recovers_via_resync` does not guarantee the `Lagged` path
  fires. Summary: the M4 convergence test (`src/server/tests/ws_convergence.rs`)
  floods 400 small events to a non-reading client to pressure a broadcast
  `Lagged` → resync, but the OS TCP buffer may absorb all 400 frames so the
  server egress never lags. The test still asserts convergence (final seq = 400,
  no dups/reordering), which holds via either live or resync delivery — so it is
  a valid convergence test but NOT a reliable regression guard for the
  lag-driven resync path specifically. Status: Needs triage — to assert the lag
  path deterministically, check `gaps_detected`/`resyncs_*`/`lagged_drops` via
  `/api/debug/rooms` (or shrink `BROADCAST_CAPACITY` under a test cfg). The
  reconnect test (`all_clients_converge_after_reconnect`) does exercise the
  resync replay path explicitly via `ResyncRequest`.
