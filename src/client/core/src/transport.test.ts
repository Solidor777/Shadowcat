import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { webSocketConnect } from "./transport";
import type { TransportHandlers } from "./transport";

/** A stub `WebSocket` recording every instance and letting the test fire the
 * lifecycle events the real browser socket would dispatch. */
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  readonly url: string;
  closed = false;
  private listeners: Record<string, Array<(ev?: unknown) => void>> = {};

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  addEventListener(type: string, cb: (ev?: unknown) => void): void {
    (this.listeners[type] ??= []).push(cb);
  }

  close(): void {
    this.closed = true;
  }

  send(): void {}

  fireOpen(): void {
    for (const cb of this.listeners.open ?? []) cb();
  }

  fireClose(): void {
    for (const cb of this.listeners.close ?? []) cb();
  }

  fireError(): void {
    for (const cb of this.listeners.error ?? []) cb();
  }

  fireMessage(data: string): void {
    for (const cb of this.listeners.message ?? []) cb({ data });
  }
}

const noopHandlers: TransportHandlers = {
  onMessage: () => {},
  onClose: () => {},
};

describe("webSocketConnect", () => {
  let originalWebSocket: unknown;

  beforeEach(() => {
    FakeWebSocket.instances = [];
    originalWebSocket = (globalThis as { WebSocket?: unknown }).WebSocket;
    (globalThis as { WebSocket?: unknown }).WebSocket = FakeWebSocket as unknown;
    vi.useFakeTimers();
  });

  afterEach(() => {
    (globalThis as { WebSocket?: unknown }).WebSocket = originalWebSocket;
    vi.useRealTimers();
  });

  it("rejects and closes the socket after connectTimeoutMs with no open", async () => {
    const connect = webSocketConnect("ws://x", 10_000);
    const onClose = vi.fn();
    const p = connect({ onMessage: () => {}, onClose });
    const rejection = expect(p).rejects.toThrow(/timeout/i);
    await vi.advanceTimersByTimeAsync(10_000);
    await rejection;
    expect(FakeWebSocket.instances[0].closed).toBe(true);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a pre-open close/error does NOT invoke handlers.onClose (no double reconnect signal)", async () => {
    const connect = webSocketConnect("ws://x", 10_000);
    const onClose = vi.fn();
    const p = connect({ onMessage: () => {}, onClose });
    const rejection = expect(p).rejects.toThrow(/error/i);
    const ws = FakeWebSocket.instances[0];
    ws.fireError();
    ws.fireClose();
    await rejection;
    expect(onClose).not.toHaveBeenCalled();
  });

  it("post-open close invokes handlers.onClose exactly once", async () => {
    const connect = webSocketConnect("ws://x", 10_000);
    const onClose = vi.fn();
    const p = connect({ onMessage: () => {}, onClose });
    const ws = FakeWebSocket.instances[0];
    ws.fireOpen();
    const transport = await p;
    ws.fireClose();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(transport).toBeDefined();
  });

  it("resolves with a working transport after open, and message frames are forwarded", async () => {
    const connect = webSocketConnect("ws://x");
    const onMessage = vi.fn();
    const p = connect({ ...noopHandlers, onMessage });
    const ws = FakeWebSocket.instances[0];
    ws.fireOpen();
    const transport = await p;
    ws.fireMessage("hello");
    expect(onMessage).toHaveBeenCalledWith("hello");
    transport.close();
    expect(ws.closed).toBe(true);
  });
});
