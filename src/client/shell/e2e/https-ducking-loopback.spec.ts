import { test } from "@playwright/test";
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import type { AddressInfo } from "node:net";
import type { Duplex } from "node:stream";
import { loginAt } from "./https-support";
import { checkMeasuredOutcome, type OutcomeTable } from "../e2e-support/platform-outcomes";

// The OS audio-session monitor is a separate local process serving `ws://127.0.0.1:<port>`. An
// `https:` page reaching it is a mixed-content request: engines exempt loopback destinations, but
// each engine applies its own rule. These tests drive a stand-in loopback WebSocket server from an
// https page in each engine and pins the measured outcome: Chromium and Firefox deliver the frame,
// WebKit blocks it (measured on the Playwright WebKit build, from both `ws://127.0.0.1` and
// `ws://localhost`, while the same connection from a plain-http page succeeds, so the block is
// the https-to-ws mixed-content rule and not the stand-in server). Real Safari has not been
// tested; the ducking settings show their hint on any https page whose OS monitor is not
// connected, because Playwright's WebKit build refuses. A pinned outcome that changes fails this
// test so the hint and the docs get revisited. Engines differ per OS, so each outcome is keyed by
// (project, platform); a cell without a measurement is an `UNMEASURED` annotation locally and fails
// on CI with the observed value, so the table is completed from CI runs.

/** The measured outcome per engine project and platform. */
const EXPECTED: OutcomeTable<string> = {
  "https-chromium": {
    win32: 'frame:{"type":"hello"}',
    linux: 'frame:{"type":"hello"}',
    darwin: 'frame:{"type":"hello"}',
  },
  "https-firefox": {
    win32: 'frame:{"type":"hello"}',
    linux: 'frame:{"type":"hello"}',
    darwin: 'frame:{"type":"hello"}',
  },
  "https-webkit": { win32: "blocked", linux: "blocked", darwin: "blocked" },
};

/**
 * A one-frame WebSocket server on loopback: completes the RFC 6455 handshake and sends one text
 * frame.
 * @returns Its port and a closer.
 * @example
 * ```
 * const loop = await loopbackWebSocket();
 * await loop.close();
 * ```
 */
async function loopbackWebSocket(): Promise<{ port: number; close: () => Promise<void> }> {
  const server = createServer();
  const sockets = new Set<Duplex>();
  server.on("upgrade", (req, socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    const accept = createHash("sha1")
      .update(`${req.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
      .digest("base64");
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    const payload = Buffer.from(JSON.stringify({ type: "hello" }));
    socket.write(Buffer.concat([Buffer.from([0x81, payload.length]), payload]));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    port: (server.address() as AddressInfo).port,
    // `server.close` waits for upgraded sockets, which never end on their own.
    close: () =>
      new Promise<void>((resolve) => {
        for (const s of sockets) s.destroy();
        server.close(() => resolve());
      }),
  };
}

test("an https page reaches the monitor loopback ws:// endpoint", async ({ page, baseURL }, testInfo) => {
  const loop = await loopbackWebSocket();
  try {
    await loginAt(page, baseURL!);
    const outcome = await page.evaluate(
      (port) =>
        new Promise<string>((resolve) => {
          const ws = new WebSocket(`ws://127.0.0.1:${port}`);
          const timer = setTimeout(() => resolve("timeout"), 10_000);
          ws.onmessage = (m) => {
            clearTimeout(timer);
            ws.close();
            resolve(`frame:${m.data}`);
          };
          ws.onerror = () => {
            clearTimeout(timer);
            resolve("blocked");
          };
        }),
      loop.port,
    );
    testInfo.annotations.push({ type: "loopback-ws-from-https", description: outcome });
    checkMeasuredOutcome(testInfo, EXPECTED, outcome);
  } finally {
    await loop.close();
  }
});
