import { describe, it, expect } from "vitest";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import WebSocket from "ws";
import { WsClient, type WireWelcome } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import { startTestServer, login, type TestServer } from "./server-process";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () =>
        resolve({
          send: (d: string) => sock.send(d),
          close: () => sock.close(),
        }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

/**
 * Reads `server_version` off the real `Welcome` frame. `/api/config` is
 * deliberately pre-auth and does not expose the version
 * (`ws/protocol.rs` `server_version` doc comment), so a WS round-trip is the
 * only clean way to read the running server's version from this test.
 */
async function fetchRunningServerVersion(server: TestServer, cookie: string): Promise<string> {
  const { world } = server.fixture;
  let welcome: WireWelcome | null = null;
  const client = new WsClient({
    connect: nodeConnect(server.wsUrl, world, cookie),
    handlers: {
      onCommand: () => {},
      onReject: () => {},
      onWelcome: (w) => {
        welcome = w;
      },
    },
  });
  await client.start();
  for (let i = 0; i < 50 && welcome === null; i++) await sleep(100);
  client.stop();
  if (welcome === null) throw new Error("did not receive Welcome within timeout");
  return (welcome as unknown as WireWelcome).server_version;
}

describe("module toolchain e2e", () => {
  it("discovers an installed module, enables it per-world, and serves its entry through the path-traversal-guarded static route", async () => {
    const modulesDir = mkdtempSync(path.join(tmpdir(), "shadowcat-modules-"));
    const modDir = path.join(modulesDir, "fixture-mod");
    mkdirSync(modDir, { recursive: true });

    const server = await startTestServer({ modulesDir });
    try {
      const cookie = await login(server.baseUrl, "gm", "pw");

      // `scan_installed_modules` runs per-request (not just at server startup),
      // so module.json can be written after the server is up — which lets the
      // fixture's engines range track the ACTUAL running server version instead
      // of a hardcoded guess.
      const serverVersion = await fetchRunningServerVersion(server, cookie);
      writeFileSync(
        path.join(modDir, "module.json"),
        JSON.stringify({
          id: "fixture-mod",
          version: "1.0.0",
          dependencies: {},
          engines: { shadowcat: `^${serverVersion}` },
        }),
      );
      writeFileSync(
        path.join(modDir, "index.js"),
        "export default { manifest: { id: 'fixture-mod', version: '1.0.0', dependencies: {} }, register() {} };\n",
      );
      // A file OUTSIDE fixture-mod/ but inside modulesDir — the traversal
      // assertion below proves the guard, not just a 404-on-missing-file.
      writeFileSync(path.join(modulesDir, "secret.txt"), "should-not-be-served");

      const list = (await fetch(`${server.baseUrl}/api/modules`, {
        headers: { cookie },
      }).then((r) => r.json())) as Array<{ manifest: { id: string }; entry_url: string }>;
      const found = list.find((m) => m.manifest.id === "fixture-mod");
      expect(found).toBeDefined();
      expect(found!.entry_url).toBe("/modules/fixture-mod/index.js");

      const entryRes = await fetch(`${server.baseUrl}${found!.entry_url}`, {
        headers: { cookie },
      });
      expect(entryRes.status).toBe(200);
      expect(entryRes.headers.get("content-type")).toContain("text/javascript");
      expect(await entryRes.text()).toContain("fixture-mod");

      // Path traversal (percent-encoded so it is not client-side-normalized
      // away before the request is even sent) is rejected.
      const traversal = await fetch(
        `${server.baseUrl}/modules/fixture-mod/%2e%2e%2fsecret.txt`,
        { headers: { cookie } },
      );
      expect(traversal.status).toBe(404);

      const enable = await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        {
          method: "PUT",
          headers: { "content-type": "application/json", cookie },
          body: JSON.stringify(["fixture-mod"]),
        },
      );
      expect(enable.status).toBe(204);

      const enabled = (await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        { headers: { cookie } },
      ).then((r) => r.json())) as string[];
      expect(enabled).toEqual(["fixture-mod"]);

      const badEnable = await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        {
          method: "PUT",
          headers: { "content-type": "application/json", cookie },
          body: JSON.stringify(["not-a-real-module"]),
        },
      );
      expect(badEnable.status).toBe(422);
    } finally {
      server.stop();
      rmSync(modulesDir, { recursive: true, force: true });
    }
  }, 30_000);
});
