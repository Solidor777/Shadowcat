import { test, expect } from "vitest";
import path from "node:path";
import { E2E_SERVER_ENV, freePort, serverBinaryPath } from "./server-process";

test("serverBinaryPath picks the debug build unless the release profile is requested", () => {
  expect(serverBinaryPath("/repo", {}, "linux")).toBe(path.join("/repo", "target", "debug", "shadowcat"));
  expect(serverBinaryPath("/repo", { SHADOWCAT_E2E_PROFILE: "release" }, "linux")).toBe(
    path.join("/repo", "target", "release", "shadowcat"),
  );
  expect(serverBinaryPath("/repo", { SHADOWCAT_E2E_PROFILE: "other" }, "linux")).toBe(
    path.join("/repo", "target", "debug", "shadowcat"),
  );
});

test("serverBinaryPath adds the executable suffix on Windows only", () => {
  expect(serverBinaryPath("/repo", {}, "win32")).toBe(path.join("/repo", "target", "debug", "shadowcat.exe"));
  expect(serverBinaryPath("/repo", {}, "darwin")).toBe(path.join("/repo", "target", "debug", "shadowcat"));
});

test("freePort yields a usable, non-zero port", async () => {
  const port = await freePort();
  expect(port).toBeGreaterThan(0);
  expect(port).toBeLessThanOrEqual(65535);
});

test("E2E_SERVER_ENV seeds the admin the specs log in as and turns the setup token off", () => {
  expect(E2E_SERVER_ENV.SHADOWCAT_ADMIN_USER).toBe("ops");
  expect(E2E_SERVER_ENV.SHADOWCAT_SETUP_TOKEN).toBe("off");
});
