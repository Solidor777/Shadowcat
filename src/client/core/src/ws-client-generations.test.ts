import { describe, it, expect, vi } from "vitest";
import { WsClient, NotConnectedError } from "./ws-client";
import type { Connect, Transport, TransportHandlers } from "./transport";

const macrotask = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** One connection attempt a test drives by hand. */
interface Attempt {
  /** The callbacks the client handed to `connect` for this attempt. */
  handlers: TransportHandlers;
  /** Frames the client sent on this attempt's transport. */
  sent: string[];
  /** How many times the client called `close()` on this attempt's transport. */
  closeCalls: number;
  /** Settles the pending `connect` with this attempt's transport. */
  resolve: () => void;
  /** Fails the pending `connect` like a refused socket. */
  reject: () => void;
}

/** A `Connect` whose attempts stay pending until the test resolves them. Its transport's
 * `close()` delivers `onClose` on a later macrotask, once, like a real socket's close event
 * (`webSocketConnect`'s transport); a second `close()` on a closed socket does nothing. */
function manualNet(): { connect: Connect; attempts: Attempt[] } {
  const attempts: Attempt[] = [];
  const connect: Connect = (handlers) =>
    new Promise<Transport>((resolve, reject) => {
      const attempt: Attempt = {
        handlers,
        sent: [],
        closeCalls: 0,
        resolve: () => resolve(transport),
        reject: () => reject(new Error("connect refused")),
      };
      let closed = false;
      const transport: Transport = {
        send: (d) => attempt.sent.push(d),
        close: () => {
          attempt.closeCalls += 1;
          if (closed) return;
          closed = true;
          setTimeout(() => handlers.onClose(), 0);
        },
      };
      attempts.push(attempt);
    });
  return { connect, attempts };
}

/** A client over `manualNet` with an observable reconnect sleep (never resolves) and a short
 * Welcome watchdog. */
function makeClient(opts: { welcomeTimeoutMs?: number } = {}) {
  const net = manualNet();
  const sleep = vi.fn(() => new Promise<void>(() => {}));
  const onConnectFailure = vi.fn();
  const client = new WsClient({
    world: "w1",
    connect: net.connect,
    handlers: { onCommand: () => {}, onConnectFailure },
    sleep,
    welcomeTimeoutMs: opts.welcomeTimeoutMs ?? 40,
  });
  return { client, sleep, onConnectFailure, ...net };
}

describe("WsClient connect generations", () => {
  it("a connect that resolves after stop()+start() is closed and never replaces the current transport", async () => {
    const { client, attempts } = makeClient();
    const first = client.start();
    client.stop();
    const second = client.start();
    expect(attempts).toHaveLength(2);
    const [a, b] = attempts;

    b.resolve();
    await second;
    a.resolve();
    await first;

    expect(a.closeCalls).toBe(1);
    client.send({ type: "pong" });
    expect(b.sent.filter((f) => f.includes('"pong"'))).toHaveLength(1);
    expect(a.sent).toHaveLength(0);
    client.stop();
  });

  it("a superseded connect that fails schedules no reconnect, reports no failure and keeps the current transport", async () => {
    const { client, attempts, sleep, onConnectFailure } = makeClient();
    const first = client.start();
    client.stop();
    const second = client.start();
    const [a, b] = attempts;

    b.resolve();
    await second;
    a.reject();
    await first;

    expect(sleep).not.toHaveBeenCalled();
    expect(onConnectFailure).not.toHaveBeenCalled();
    client.send({ type: "pong" });
    expect(b.sent.filter((f) => f.includes('"pong"'))).toHaveLength(1);
    expect(b.closeCalls).toBe(0);
    client.stop();
  });

  it("a connect that fails after a plain stop() schedules no reconnect and reports no failure", async () => {
    const { client, attempts, sleep, onConnectFailure } = makeClient();
    const first = client.start();
    client.stop();
    attempts[0].reject();
    await first;

    expect(sleep).not.toHaveBeenCalled();
    expect(onConnectFailure).not.toHaveBeenCalled();
  });
});

/** Starts A, replaces it by stop()+start() with B, adopts B, then lets A's late close event
 * arrive. Returns once A's close has been delivered. */
async function lateCloseOfReplacedTransport() {
  const ctx = makeClient();
  const { client, attempts } = ctx;
  const first = client.start();
  attempts[0].resolve();
  await first;
  client.stop();
  const second = client.start();
  attempts[1].resolve();
  await second;
  const pending = client.search("q", { timeoutMs: 60_000 }).then(
    () => "resolved",
    (e: unknown) => (e instanceof NotConnectedError ? "failed" : "other"),
  );
  let settled: string | null = null;
  void pending.then((s) => (settled = s));
  await macrotask(); // A's close event, queued by stop(), is delivered here
  return { ...ctx, getSettled: () => settled };
}

describe("a replaced transport's late close does not act on the current connection", () => {
  it("keeps the current transport", async () => {
    const { client, attempts } = await lateCloseOfReplacedTransport();
    client.send({ type: "pong" });
    expect(attempts[1].sent.filter((f) => f.includes('"pong"'))).toHaveLength(1);
    client.stop();
  });

  it("keeps the current connection's Welcome watchdog armed", async () => {
    const { client, attempts } = await lateCloseOfReplacedTransport();
    await new Promise((r) => setTimeout(r, 120));
    expect(attempts[1].closeCalls).toBe(1);
    client.stop();
  });

  it("does not fail the current connection's pending requests", async () => {
    const { client, getSettled } = await lateCloseOfReplacedTransport();
    expect(getSettled()).toBeNull();
    client.stop();
  });

  it("schedules no reconnect", async () => {
    const { client, sleep } = await lateCloseOfReplacedTransport();
    expect(sleep).not.toHaveBeenCalled();
    client.stop();
  });
});

describe("the current transport's close", () => {
  it("runs the full close handling", async () => {
    const { client, attempts, sleep } = makeClient();
    const started = client.start();
    attempts[0].resolve();
    await started;
    const pending = client.search("q", { timeoutMs: 60_000 }).then(
      () => "resolved",
      (e: unknown) => (e instanceof NotConnectedError ? "failed" : "other"),
    );

    attempts[0].handlers.onClose();

    expect(await pending).toBe("failed");
    expect(sleep).toHaveBeenCalledTimes(1);
    client.send({ type: "pong" });
    expect(attempts[0].sent.filter((f) => f.includes('"pong"'))).toHaveLength(0);
    await new Promise((r) => setTimeout(r, 120));
    expect(attempts[0].closeCalls).toBe(0);
    client.stop();
  });
});
