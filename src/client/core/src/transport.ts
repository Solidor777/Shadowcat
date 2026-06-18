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
 * needs a cookie header supplies its own `Connect` instead. */
export function webSocketConnect(url: string): Connect {
  return (handlers) =>
    new Promise<Transport>((resolve, reject) => {
      const ws = new WebSocket(url);
      ws.addEventListener("open", () => {
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
      ws.addEventListener("close", () => handlers.onClose());
      ws.addEventListener("error", () => reject(new Error("websocket error")));
    });
}
