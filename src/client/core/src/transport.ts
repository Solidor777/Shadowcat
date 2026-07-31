// Transport abstraction so the WS client is testable without a real socket.
// Production supplies a `WebSocket`-backed connector; tests supply an in-memory
// paired connector (see mock-server.ts).

export interface Transport {
  /** Send a text frame. */
  send(data: string): void;
  /** Close the connection (triggers `onClose`). */
  close(): void;
}

export interface TransportHandlers {
  onMessage(data: string): void;
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
 * pre-open onClose would double-schedule it). */
export function webSocketConnect(url: string, connectTimeoutMs = 10_000): Connect {
  return (handlers) =>
    new Promise<Transport>((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      const timer = setTimeout(() => {
        if (!opened) {
          reject(new Error("websocket connect timeout"));
          ws.close();
        }
      }, connectTimeoutMs);
      ws.addEventListener("open", () => {
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
          reject(new Error("websocket error"));
        }
        // Post-open errors are followed by `close`; onClose handles teardown.
      });
    });
}
