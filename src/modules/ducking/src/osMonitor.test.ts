// @vitest-environment node
import { describe, it, expect, vi, afterEach } from "vitest";
import { OsMonitorSource } from "./osMonitor";

/** A scriptable fake `WebSocket` capturing the handlers `OsMonitorSource` assigns, with a
 * `simulate*` helper per frame kind so tests drive the connection without a real socket. */
class FakeSocket {
  static OPEN = 1;
  static CONNECTING = 0;
  readyState = FakeSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  sent: string[] = [];
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
    this.readyState = 3;
  }
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }
  message(payload: unknown): void {
    this.onmessage?.({ data: JSON.stringify(payload) });
  }
}

const logger = { debug() {}, warn() {}, error() {} };

describe("OsMonitorSource", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("sends the initial watch list once connected", () => {
    let socket!: FakeSocket;
    const source = new OsMonitorSource({
      port: 31998,
      watch: ["discord"],
      logger,
      createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
    });
    source.start();
    socket.open();
    expect(socket.sent).toEqual([JSON.stringify({ type: "watch", names: ["discord"] })]);
    source.stop();
  });

  it("reports status from the hello frame", () => {
    let socket!: FakeSocket;
    const source = new OsMonitorSource({
      port: 31998,
      watch: [],
      logger,
      createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
    });
    const statuses: string[] = [];
    source.onStatusChange((s) => statuses.push(s));
    source.start();
    socket.open();
    socket.message({ type: "hello", os: "linux", supported: false, reason: "PipeWire not running" });
    expect(source.getStatus()).toBe("unsupported");
    expect(statuses).toContain("unsupported");
    source.stop();
  });

  it("sets demand 1 when a session's peak exceeds threshold, with hangover on drop", () => {
    vi.useFakeTimers();
    let socket!: FakeSocket;
    const calls: number[] = [];
    const source = new OsMonitorSource({
      port: 31998,
      watch: ["discord"],
      logger,
      createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
    });
    source.setSink({ set: (v) => calls.push(v) });
    source.start();
    socket.open();
    socket.message({ type: "levels", sessions: [{ process: "discord", peak: 0.5 }] });
    expect(calls).toEqual([1]);
    socket.message({ type: "levels", sessions: [{ process: "discord", peak: 0.0 }] });
    expect(calls).toEqual([1]); // still in hangover
    vi.advanceTimersByTime(400);
    expect(calls).toEqual([1, 0]);
    source.stop();
  });

  it("setWatch sends a live watch frame while connected", () => {
    let socket!: FakeSocket;
    const source = new OsMonitorSource({
      port: 31998,
      watch: [],
      logger,
      createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
    });
    source.start();
    socket.open();
    socket.sent = [];
    source.setWatch(["firefox"]);
    expect(socket.sent).toEqual([JSON.stringify({ type: "watch", names: ["firefox"] })]);
    source.stop();
  });

  it("reports not-running after 3 failed connection attempts", () => {
    vi.useFakeTimers();
    const sockets: FakeSocket[] = [];
    const source = new OsMonitorSource({
      port: 31998,
      watch: [],
      logger,
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s as unknown as WebSocket;
      },
    });
    const statuses: string[] = [];
    source.onStatusChange((s) => statuses.push(s));
    source.start();
    for (let i = 0; i < 3; i += 1) {
      sockets.at(-1)?.onclose?.();
      vi.runOnlyPendingTimers();
    }
    expect(statuses.at(-1)).toBe("not-running");
    source.stop();
  });
});
