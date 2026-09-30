import { describe, it, expect } from "vitest";
import { WsClient, NotConnectedError, RequestTimeoutError } from "./ws-client";

// Every correlated-request path rejects with a TYPED error, so a caller tells "the link is down"
// from "the server did not answer" by `instanceof` and never by message text. Each table below
// covers every site of one rejection class; reverting one site to a plain `Error` turns exactly
// that row red.

type Call = (client: WsClient, opts: { timeoutMs: number }) => Promise<unknown>;

/** Requests with a caller-supplied timeout: they have all three rejection classes. */
const TIMED: [string, Call][] = [
  ["search", (c, o) => c.search("x", o)],
  ["subscribeSearch", (c, o) => c.subscribeSearch("x", o, () => {})],
  ["subscribeScene", (c, o) => c.subscribeScene("identity", () => {}, o)],
  ["pathfind", (c, o) => c.pathfind("s1", [0, 0], [[1, 1]], 0.5, undefined, o)],
  ["moveRequest", (c, o) => c.moveRequest("s1", "t1", [[0, 0], [1, 1]], o)],
  ["combat", (c, o) => c.combat({ type: "combat_start" } as never, o)],
  ["merge", (c, o) => c.merge({ type: "merge_pull" } as never, o)],
];

/** In-flight requests without a timeout option that `failPending` also rejects on a closed socket. */
const UNTIMED: [string, Call][] = [
  ["sendChatMessage", (c) => c.sendChatMessage({ channel: "main", content: "hi" } as never)],
  ["toggleDoor", (c) => c.toggleDoor("s1", "w1", "open")],
];

/** A started client whose socket the test can close, with reconnection parked forever. */
async function startedClient(): Promise<{ client: WsClient; closeSocket: () => void }> {
  let onClose: () => void = () => {};
  const client = new WsClient({
    world: "w1",
    connect: (h) => {
      onClose = h.onClose;
      return Promise.resolve({ send: () => {}, close: () => {} });
    },
    handlers: { onCommand: () => {} },
    sleep: () => new Promise<void>(() => {}),
  });
  await client.start();
  return { client, closeSocket: () => onClose() };
}

/** A client that was never started, so it holds no transport. */
function idleClient(): WsClient {
  return new WsClient({
    world: "w1",
    connect: () => Promise.reject(new Error("unused")),
    handlers: { onCommand: () => {} },
  });
}

describe("typed rejections of correlated requests", () => {
  it.each(TIMED)("%s without a transport rejects with NotConnectedError", async (_name, call) => {
    await expect(call(idleClient(), { timeoutMs: 60_000 })).rejects.toBeInstanceOf(NotConnectedError);
  });

  it.each(TIMED)("%s with no reply in time rejects with RequestTimeoutError, not NotConnectedError", async (_name, call) => {
    const { client } = await startedClient();
    const err = await call(client, { timeoutMs: 1 }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(RequestTimeoutError);
    expect(err).not.toBeInstanceOf(NotConnectedError);
  });

  it.each([...TIMED, ...UNTIMED])("%s in flight when the socket closes rejects with NotConnectedError", async (_name, call) => {
    const { client, closeSocket } = await startedClient();
    const pending = call(client, { timeoutMs: 60_000 });
    const settled = pending.catch((e: unknown) => e);
    closeSocket();
    const err = await settled;
    expect(err).toBeInstanceOf(NotConnectedError);
    expect(err).not.toBeInstanceOf(RequestTimeoutError);
  });

  it.each([...TIMED, ...UNTIMED])("%s in flight when the client stops rejects with NotConnectedError", async (_name, call) => {
    const { client } = await startedClient();
    const settled = call(client, { timeoutMs: 60_000 }).catch((e: unknown) => e);
    client.stop();
    expect(await settled).toBeInstanceOf(NotConnectedError);
  });
});
