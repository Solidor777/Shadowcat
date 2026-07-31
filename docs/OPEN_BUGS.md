# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

Silent-hang startup paths (2026-07-31 render-ready audit; each converts a
transient failure into a permanent, error-free dead-end UI):

- [Hang] No Welcome watchdog: `WorldSession.enter` awaits the server's
  `Welcome` frame with no timeout, retry, or error state
  (`worldSession.svelte.ts` — `role` is only set in `#onWelcome`, and
  `App.svelte`'s world gate renders "Connecting…" until then). The browser's
  socket `open` fires at HTTP 101, BEFORE the server's Welcome preamble
  (~9 DB round trips + a blocking `scan_installed_modules` fs scan per
  connect, `ws/conn.rs`), so a stalled preamble leaves the client on
  "Connecting…" forever — reconnect machinery only reacts to socket CLOSE.
  Every other correlated WS request has a 10s timeout (`ws-client.ts`);
  Welcome is the one exception. Fix direction: arm a 10s watchdog after
  `#ws.start()`, cleared in `#onWelcome`; on expiry force a reconnect or a
  retryable error state.
- [Hang] `boot()`'s three fetches (`getMe`, `getUiState`, `listWorlds` —
  `App.svelte`/`api.ts`) are unbounded and unretried; any transient non-2xx
  or connection reset permanently degrades to the login or worlds route
  with no visible error and no retry. Fix direction: `AbortSignal.timeout`
  on the fetch helpers + a small bounded retry in `boot()` before
  degrading.
- [Hang] `webSocketConnect` (`client/core/src/transport.ts`) settles only on
  the socket's `open`/`error` events — a TCP-accepted-but-never-upgraded
  handshake never settles, and `ws-client.ts`'s reconnect path is
  unreachable behind the unsettled await. Fix direction: a connect timeout
  that rejects and closes the socket so `scheduleReconnect` runs.
- [Wedge] `WorldSession.#bootstrapped` latches `true` BEFORE
  `await #modules.activate()` (`worldSession.svelte.ts`), so a failed or
  hung first activation (e.g. a manifest dependency cycle throwing out of
  `topoSort`) is cached for the session's life: reconnect Welcomes
  short-circuit, `role` is set, the Table mounts, but every Surface stays
  empty and `.stage-host` never appears — logged only. Fix direction:
  tri-state the latch (pending/done/failed) so the next Welcome retries
  activation.
