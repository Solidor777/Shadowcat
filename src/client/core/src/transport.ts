// Transport abstraction so the WS client is testable without a real socket.
// Production supplies a `WebSocket`-backed connector; tests supply an in-memory
// paired connector (see mock-server.ts).

/** The connector surface `WsClient` sends/closes through, independent of the
 * backing implementation (real `WebSocket` or an in-memory test pair). */
export interface Transport {
  /** Send a text frame.
   * @param data The frame payload to send.
   */
  send(data: string): void;
  /** Close the connection (triggers `onClose`). */
  close(): void;
}

/** Callbacks `Connect` invokes for events on the opened `Transport`. */
export interface TransportHandlers {
  /** Called with each inbound text frame.
   * @param data The received frame payload.
   */
  onMessage(data: string): void;
  /** Called once the transport closes, after a successful open. */
  onClose(): void;
}

/** Open a connection, resolving once it is ready to send/receive. The client
 * calls this again (after backoff) to reconnect, so each call is a fresh link. */
export type Connect = (handlers: TransportHandlers) => Promise<Transport>;

/** A `Connect` backed by the platform global `WebSocket` (browser / Node 22+).
 * Cookies are sent automatically by the browser; Node test/integration code that
 * needs a cookie header supplies its own `Connect` instead. `connectTimeoutMs`
 * bounds the handshake: a TCP-accepted-but-never-upgraded socket otherwise
 * never settles this promise, and the caller's reconnect machinery is
 * unreachable behind the unsettled await. Handlers attach semantically AFTER
 * open: pre-open close/error only reject (they must not leak into onClose —
 * the caller's open() failure path already schedules the reconnect, and a
 * pre-open onClose would double-schedule it).
 * @param url The WebSocket URL to connect to.
 * @param connectTimeoutMs Bounds the handshake; a TCP-accepted-but-never-upgraded socket rejects (and is closed) after this many ms instead of hanging forever.
 * @returns A `Connect` function suitable for `WsClient`.
 * @example
 * ```ts
 * import { webSocketConnect } from "@shadowcat/core";
 *
 * const connect = webSocketConnect("ws://localhost/ws");
 * ```
 */
export function webSocketConnect(url: string, connectTimeoutMs = 10_000): Connect {
  return (handlers) =>
    new Promise<Transport>((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      // Settled once the connect PROMISE has resolved or rejected (timeout,
      // error, or a completed open) — distinct from `opened`, which only
      // tracks the socket's own open/not-open state. A `connectTimeoutMs`
      // expiry and an already-queued `open` event can land in the same tick;
      // without this guard `opened` could flip true AFTER the promise already
      // rejected, letting a later `close` still call `handlers.onClose()` —
      // the caller's `open()` catch AND `handleClose()` would then both
      // schedule a reconnect (two live transports), and the orphan's own
      // close would null out `this.transport` from under the live one.
      let settled = false;
      const timer = setTimeout(() => {
        if (!opened) {
          settled = true;
          reject(new Error("websocket connect timeout"));
          ws.close();
        }
      }, connectTimeoutMs);
      ws.addEventListener("open", () => {
        if (settled) {
          // The connect promise already settled (e.g. the timeout fired in
          // the same tick this `open` was already queued) — discard this
          // socket without ever reaching `opened`/`onClose`.
          ws.close();
          return;
        }
        settled = true;
        opened = true;
        clearTimeout(timer);
        resolve({
          send: (data) => ws.send(data),
          close: () => ws.close(),
        });
      });
      ws.addEventListener("message", (ev: MessageEvent) => {
        handlers.onMessage(
          typeof ev.data === "string" ? ev.data : String(ev.data),
        );
      });
      ws.addEventListener("close", () => {
        clearTimeout(timer);
        if (opened) handlers.onClose();
      });
      ws.addEventListener("error", () => {
        if (!opened) {
          clearTimeout(timer);
          settled = true;
          reject(new Error("websocket error"));
        }
        // Post-open errors are followed by `close`; onClose handles teardown.
      });
    });
}
