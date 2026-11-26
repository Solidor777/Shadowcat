import { test, expect } from "vitest";
import path from "node:path";
import { fileURLToPath } from "node:url";
import config from "../playwright.config";
import {
  E2E_OUTPUT_DIR,
  E2E_SERVER_DATA_DIR,
  E2E_SERVER_ENV,
  freePort,
  serverBinaryPath,
  startedServerEnv,
} from "./server-process";

/** The repository root, four folders above this file. */
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");

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

/** Whether `child` is `parent` or inside it. */
function isInside(parent: string, child: string): boolean {
  const relative = path.relative(parent, child);
  return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
}

/** Asserts `env` names an assets folder and a trash folder inside `E2E_SERVER_DATA_DIR`, as siblings. */
function expectWrittenFoldersContained(env: Record<string, string | undefined>, label: string): void {
  for (const key of ["SHADOWCAT_ASSETS_DIR", "SHADOWCAT_TRASH_DIR"]) {
    const dir = env[key];
    expect(dir, `${label} sets ${key}`).toBeDefined();
    expect(path.isAbsolute(dir as string), `${label} ${key} is absolute`).toBe(true);
    expect(isInside(E2E_SERVER_DATA_DIR, dir as string), `${label} ${key} is inside the test-data folder`).toBe(true);
  }
  expect(isInside(env.SHADOWCAT_ASSETS_DIR as string, env.SHADOWCAT_TRASH_DIR as string), `${label} trash nests in assets`).toBe(false);
  expect(isInside(env.SHADOWCAT_TRASH_DIR as string, env.SHADOWCAT_ASSETS_DIR as string), `${label} assets nests in trash`).toBe(false);
}

test("the test-data folder is inside Playwright's cleared output folder, never the repo root", () => {
  expect(isInside(config.outputDir as string, E2E_SERVER_DATA_DIR)).toBe(true);
  expect(config.outputDir).toBe(E2E_OUTPUT_DIR);
  expect(isInside(REPO_ROOT, E2E_OUTPUT_DIR)).toBe(true);
  expect(path.dirname(E2E_SERVER_DATA_DIR)).not.toBe(REPO_ROOT);
});

test("every webServer of the Playwright config keeps its uploads and trash in its own test-data folder", () => {
  const servers = [config.webServer ?? []].flat();
  expect(servers.length).toBeGreaterThanOrEqual(5);
  const assetsDirs = new Set<string>();
  for (const server of servers) {
    const env = server.env ?? {};
    expectWrittenFoldersContained(env, env.SHADOWCAT_BIND ?? "webServer");
    assetsDirs.add(env.SHADOWCAT_ASSETS_DIR as string);
  }
  expect(assetsDirs.size, "no two servers share an asset store").toBe(servers.length);
});

test("a server a spec starts itself keeps its database, uploads and trash in the given data folder", () => {
  const dataDir = path.join(E2E_SERVER_DATA_DIR, "started");
  const env = startedServerEnv({ repoRoot: REPO_ROOT, dataDir }, 41234);
  expectWrittenFoldersContained(env, "startServer");
  expect(env.SHADOWCAT_DB).toContain("shadowcat.db");
  expect(env.SHADOWCAT_BIND).toBe("127.0.0.1:41234");
});

test("E2E_SERVER_ENV seeds the admin the specs log in as and turns the setup token off", () => {
  expect(E2E_SERVER_ENV.SHADOWCAT_ADMIN_USER).toBe("ops");
  expect(E2E_SERVER_ENV.SHADOWCAT_SETUP_TOKEN).toBe("off");
});
