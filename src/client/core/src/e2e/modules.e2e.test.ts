import { describe, it, expect } from "vitest";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { startTestServer, login } from "./server-process";

describe("module toolchain e2e", () => {
  it("discovers an installed module, enables it per-world, and serves its entry through the path-traversal-guarded static route", async () => {
    const modulesDir = mkdtempSync(path.join(tmpdir(), "shadowcat-modules-"));
    const modDir = path.join(modulesDir, "fixture-mod");
    mkdirSync(modDir, { recursive: true });
    writeFileSync(
      path.join(modDir, "module.json"),
      JSON.stringify({
        id: "fixture-mod",
        version: "1.0.0",
        dependencies: {},
        engines: { shadowcat: "^0.1.0" },
      }),
    );
    writeFileSync(
      path.join(modDir, "index.js"),
      "export default { manifest: { id: 'fixture-mod', version: '1.0.0', dependencies: {} }, register() {} };\n",
    );
    // A file OUTSIDE fixture-mod/ but inside modulesDir — the traversal
    // assertion below proves the guard, not just a 404-on-missing-file.
    writeFileSync(path.join(modulesDir, "secret.txt"), "should-not-be-served");

    const server = await startTestServer({ modulesDir });
    try {
      const cookie = await login(server.baseUrl, "gm", "pw");

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
    }
  }, 30_000);
});
