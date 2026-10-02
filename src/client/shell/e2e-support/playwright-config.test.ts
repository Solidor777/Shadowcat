// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NetworkInterfaceInfo } from "node:os";
import type { PlaywrightTestConfig } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { ANY_HTTPS_SPEC, HTTPS_SPECS, LAN_SPEC, MEDIA_RANGE_SPEC, UPGRADE_CHROMIUM_SPEC, UPGRADE_SPEC } from "./spec-matchers";

// Addresses are from the RFC 5737 documentation range 192.0.2.0/24.

type Interfaces = Record<string, NetworkInterfaceInfo[]>;

const LOOPBACK_ONLY: Interfaces = {
  lo: [{ address: "127.0.0.1", family: "IPv4", internal: true } as NetworkInterfaceInfo],
};
const WITH_LAN: Interfaces = {
  ...LOOPBACK_ONLY,
  phys0: [{ address: "192.0.2.20", family: "IPv4", internal: false } as NetworkInterfaceInfo],
};

/** Imports a fresh copy of `playwright.config.ts` as if the machine had `interfaces` and the
 * environment held `env`; the config reads both at import time. */
async function loadConfig(interfaces: Interfaces, env: Record<string, string | undefined>): Promise<PlaywrightTestConfig> {
  vi.resetModules();
  vi.doMock("node:os", async (original) => ({
    ...(await original<typeof import("node:os")>()),
    networkInterfaces: () => interfaces,
  }));
  for (const [key, value] of Object.entries(env)) {
    if (value === undefined) vi.stubEnv(key, undefined as unknown as string);
    else vi.stubEnv(key, value);
  }
  const module = (await import("../playwright.config")) as { default: PlaywrightTestConfig };
  return module.default;
}

function names(config: PlaywrightTestConfig): string[] {
  return (config.projects ?? []).map((p) => p.name ?? "");
}

afterEach(() => {
  vi.doUnmock("node:os");
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

describe("playwright.config LAN address handling", () => {
  it("keeps the loopback, media-range-loopback and lan-plain-http projects and drops every HTTPS project on a host with no LAN address, warning loudly", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const config = await loadConfig(LOOPBACK_ONLY, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    expect(names(config)).toEqual(["loopback", "media-range-loopback", "lan-plain-http"]);
    expect(config.webServer).toHaveLength(3);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toContain("HTTPS e2e projects OMITTED");
  });

  it("throws on CI when no LAN address exists, because the HTTPS projects are part of the gate", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    await expect(loadConfig(LOOPBACK_ONLY, { CI: "true", SHADOWCAT_E2E_LAN_IP: undefined })).rejects.toThrow();
  });

  it("adds the HTTPS projects and their server when the host has a LAN address", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const all = names(config);
    expect(all.slice(0, 3)).toEqual(["loopback", "media-range-loopback", "lan-plain-http"]);
    expect(all).toContain("https-chromium");
    expect(all.some((n) => n.startsWith("https-upgrade"))).toBe(true);
    expect(config.webServer).toHaveLength(4);
  });

  it("gives the media-range project its own server with the short egress window, and no other server has one", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const project = (config.projects ?? []).find((p) => p.name === "media-range-loopback");
    const baseURL = String(project?.use?.baseURL);
    const servers = [config.webServer ?? []].flat();
    const windowed = servers.filter((s) => s.env?.SHADOWCAT_EGRESS_WINDOW_SECS !== undefined);
    expect(windowed).toHaveLength(1);
    expect(windowed[0].env?.SHADOWCAT_EGRESS_WINDOW_SECS).toBe("3");
    // The project targets exactly that server's bind address and nothing else's.
    const bind = String(windowed[0].env?.SHADOWCAT_BIND);
    expect(baseURL).toBe(`http://${bind}`);
    for (const other of (config.projects ?? []).filter((p) => p.name !== "media-range-loopback")) {
      expect(String(other.use?.baseURL ?? "")).not.toBe(baseURL);
    }
  });
});

/** The `--project` patterns each CI job passes to `playwright test`, read from the workflow file. */
function ciProjectPatterns(): Map<string, string[]> {
  const workflow = readFileSync(fileURLToPath(new URL("../../../../.github/workflows/ci.yml", import.meta.url)), "utf8");
  const byJob = new Map<string, string[]>();
  let job = "";
  for (const line of workflow.split(/\r?\n/)) {
    const header = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line);
    if (header) job = header[1];
    if (!/playwright test\b/.test(line)) continue;
    const patterns = [...line.matchAll(/--project=(?:"([^"]+)"|'([^']+)'|(\S+))/g)].map((m) => m[1] ?? m[2] ?? m[3]);
    byJob.set(job, [...(byJob.get(job) ?? []), ...patterns]);
  }
  return byJob;
}

/** Playwright matches a `--project` value as a glob where `*` spans any characters. */
function matches(pattern: string, name: string): boolean {
  const source = pattern.split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*");
  return new RegExp(`^${source}$`).test(name);
}

describe("CI project partition", () => {
  it("runs every Playwright project in exactly one CI job", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const jobs = ciProjectPatterns();
    expect([...jobs.keys()].sort(), "both e2e jobs are read from ci.yml").toEqual(["ui-e2e", "ui-e2e-https"]);
    for (const name of names(config)) {
      const runners = [...jobs].filter(([, patterns]) => patterns.some((p) => matches(p, name))).map(([job]) => job);
      expect(runners, `project ${name} must run in exactly one CI job`).toHaveLength(1);
    }
  });

  it("names no CI project pattern that matches nothing", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    for (const [job, patterns] of ciProjectPatterns()) {
      for (const pattern of patterns) {
        expect(names(config).some((n) => matches(pattern, n)), `${job}: --project=${pattern}`).toBe(true);
      }
    }
  });
});

type ProjectPatterns = NonNullable<PlaywrightTestConfig["projects"]>[number]["testMatch"];

/** Whether `file` matches one `testMatch` / `testIgnore` value. Only the regular-expression form
 * the config uses is evaluated; a glob string would be silently mis-read, so it fails loudly. */
function patternMatches(patterns: ProjectPatterns, file: string): boolean {
  return [patterns ?? []].flat().some((pattern) => {
    if (pattern instanceof RegExp) return pattern.test(file);
    throw new Error(`playwright.config uses a non-RegExp test pattern (${String(pattern)}); teach patternMatches about it`);
  });
}

/** The projects that run the end-to-end test file `file` (a path under `e2e/`). */
function projectsRunning(config: PlaywrightTestConfig, file: string): string[] {
  return (config.projects ?? [])
    .filter((p) => (p.testMatch === undefined || patternMatches(p.testMatch, file)) && !patternMatches(p.testIgnore, file))
    .map((p) => p.name ?? "");
}

/** The end-to-end test files under `e2e/`, recursively, as paths relative to the shell package. */
function listSpecs(): string[] {
  const root = fileURLToPath(new URL("../e2e", import.meta.url));
  return readdirSync(root, { recursive: true, encoding: "utf8" })
    .map((p) => p.replace(/\\/g, "/"))
    .filter((p) => p.endsWith(".spec.ts"))
    .sort()
    .map((p) => `e2e/${p}`);
}

/** The projects whose `testMatch` is exactly `matcher`. */
function projectsMatching(config: PlaywrightTestConfig, matcher: RegExp): string[] {
  // Compared by source: the config module is re-imported per test, so its regexes are fresh objects.
  return (config.projects ?? [])
    .filter((p) => p.testMatch instanceof RegExp && p.testMatch.source === matcher.source)
    .map((p) => p.name ?? "");
}

describe("spec-to-project assignment", () => {
  const specs = listSpecs();

  it("finds the e2e specs, nested ones included", () => {
    expect(specs.length).toBeGreaterThan(10);
  });

  it("assigns the https matchers to disjoint spec sets", () => {
    for (const spec of specs) {
      const matched = [HTTPS_SPECS, UPGRADE_SPEC, UPGRADE_CHROMIUM_SPEC].filter((m) => m.test(spec));
      expect(matched.length, `${spec} matches more than one https matcher`).toBeLessThanOrEqual(1);
      expect(ANY_HTTPS_SPEC.some((m) => m.test(spec)), spec).toBe(matched.length === 1);
    }
  });

  it("runs every spec in EXACTLY the project set its matcher derives: none twice, none nowhere", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    for (const spec of specs) {
      let expected: string[];
      if (LAN_SPEC.test(spec)) expected = ["lan-plain-http"];
      else if (MEDIA_RANGE_SPEC.test(spec)) expected = ["media-range-loopback"];
      else if (HTTPS_SPECS.test(spec)) expected = projectsMatching(config, HTTPS_SPECS);
      else if (UPGRADE_CHROMIUM_SPEC.test(spec)) expected = projectsMatching(config, UPGRADE_CHROMIUM_SPEC);
      else if (UPGRADE_SPEC.test(spec)) expected = projectsMatching(config, UPGRADE_SPEC);
      else expected = ["loopback"];
      expect(expected, `no project is defined for the matcher that claims ${spec}`).not.toHaveLength(0);
      expect(projectsRunning(config, spec).sort(), `${spec} runs in the wrong projects`).toEqual([...expected].sort());
    }
  });

  it("keeps the loopback, lan-plain-http and https-* families disjoint: a spec never runs in two families", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    for (const spec of specs) {
      const runners = projectsRunning(config, spec);
      const families = new Set(runners.map((n) => (n.startsWith("https-") ? "https" : n)));
      expect(families.size, `${spec} runs in ${runners.join(", ")}`).toBe(1);
    }
  });
});
