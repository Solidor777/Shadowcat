# Shadowcat patch to h2 0.4.19

This directory is the crates.io tarball of `h2` 0.4.19 (SHA-256
`ef8e5e5a340588f4452631496976cf8636d4a7ecf600239fdc27615d2530bc16`) without tarball metadata
(`.cargo-ok`, `.cargo_vcs_info.json`, `.gitignore`, `.github/`, `Cargo.lock`). The workspace root
`Cargo.toml` routes every `h2` consumer here through `[patch.crates-io]` and excludes the
directory from the workspace members, so `cargo fmt`, `cargo clippy` and the doctests never run
over it.

## Changes to upstream

Four changes in six files: two arms in `src/proto/streams/state.rs` (connection end), one request extension in `src/server.rs` (stream id), and, in `src/proto/connection.rs` and `src/proto/streams/{counts,recv,streams}.rs`, backpressure on pending-accept resets plus a sustained reset-rate rule.

The arms:

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
server crate. The stream-id extension is covered by
`src/server/test-support/tests/vendored_h2_stream_id.rs`.

### Stream id on the request

`Peer::convert_poll_message` in `src/server.rs` adds `h2::StreamId::from_internal(stream_id)` to
every server request's extensions. `h2::StreamId` is the public type (`u32::from` reads it), so a
service that receives the request through hyper, which forwards h2's request extensions untouched,
can correlate the request with the wire frames of its stream. hyper does not expose the stream id
otherwise.

### Pending-accept reset backpressure and the reset-rate rule

Upstream counts the streams a peer has reset before the service accepted them
(`max_pending_accept_reset_streams`) and ends the connection with GOAWAY(ENHANCE_YOUR_CALM,
`too_many_resets`) when a reset would exceed the limit. A client whose HEADERS and RST_STREAM frames
reach the server in one flush (a page cancelling its requests, a stalled uplink that drains at
once) has all of them pending at once, so no finite limit is both safe and sufficient.

For a server connection:

- `Connection::poll2` stops reading frames (`Streams::pause_for_accepts`, `Counts::remote_resets_at_bound`)
  while the count equals the limit, and `Streams::next_incoming` wakes it when an accept lowers
  the count. The count therefore never exceeds the limit (a limit of 0 is treated as 1), no reset is
  refused for queueing, and the memory pending reset streams pin stays bounded by the limit. Accepting
  reads only the accept queue (`Recv::next_incoming`), never a frame or a handler, so a stopped
  connection resumes on the next accept; hyper's `poll_accept` loop pops a stream right after the
  connection reports pending. Client connections are unchanged (a client that never accepts a pushed
  stream would otherwise stop reading).
- `Counts::record_remote_reset`, called from `Recv::recv_reset` for every RST_STREAM that reaches a
  stream, charges a generic cell rate limit (`ResetRate`): a burst of up to 16 times the limit is
  admitted at once, and the sustained rate is that many resets per 10 s window
  (`REMOTE_RESET_RATE_FACTOR`, `REMOTE_RESET_RATE_WINDOW`). Exceeding it ends the connection with
  GOAWAY(ENHANCE_YOUR_CALM, `too_many_resets`). This is the HTTP/2 rapid-reset protection: opening and
  cancelling streams never holds the concurrent-stream limit, so only the reset rate bounds the server
  work a peer can demand. The numbers derive from the one reset limit hyper forwards.

The reproducers are the server crate's `http::conn_serve::reset_guard_tests`.

## Maintenance

On every `h2` release, check whether it contains an equivalent connection-end fix, a stream-id
request extension and reset backpressure with a sustained reset-rate rule; if it does, drop `vendor/h2` and the `[patch.crates-io]` entry.
