# Shadowcat patch to h2 0.4.19

This directory is the crates.io tarball of `h2` 0.4.19 (SHA-256
`ef8e5e5a340588f4452631496976cf8636d4a7ecf600239fdc27615d2530bc16`) without tarball metadata
(`.cargo-ok`, `.cargo_vcs_info.json`, `.gitignore`, `.github/`, `Cargo.lock`). The workspace root
`Cargo.toml` routes every `h2` consumer here through `[patch.crates-io]` and excludes the
directory from the workspace members, so `cargo fmt`, `cargo clippy` and the doctests never run
over it.

## Changes to upstream

Two arms in `src/proto/streams/state.rs`:

- `State::recv_eof` converts `Closed(Cause::ScheduledLibraryReset(_))` into
  `Closed(Cause::Error(<BrokenPipe>))`.
- `State::handle_error` converts `Closed(Cause::ScheduledLibraryReset(_))` into
  `Closed(Cause::Error(<the connection error>))`.

### Why

A stream whose response was ended while its DATA was queued behind a zero send window, with the
request still open, is left in `Closed(ScheduledLibraryReset(NO_ERROR))` when the last handle is
dropped. The reset can never be written once the connection has ended, and upstream never converts
that state, so `Counts::transition_after` never releases the stream's slot (it releases a counted
stream only when the state is not a scheduled reset). `Prioritize::clear_pending_send`, the only
other converter, skips a stream that is not in the connection's pending-send queue, which is exactly
a stream blocked on a zero window. The stream stays counted and, in a debug build, `Counts::drop`
fails `assert!(!self.has_streams())` when the connection is dropped.

`State::recv_eof` covers the connection ending by end-of-file and the sweep that runs when the
connection is dropped. `State::handle_error` covers the connection-level failure path: its sweep
removes an unreferenced stream from the store once its queues are cleared, so no later sweep could
find it.

The reproducers are `src/server/test-support/tests/vendored_h2_connection_end.rs` (debug builds
only: the assertion exists only under `debug_assertions`) and the drain-kill integration test in the
server crate.

## Maintenance

On every `h2` release, check whether it contains an equivalent connection-end fix; if it does, drop
`vendor/h2` and the `[patch.crates-io]` entry.
