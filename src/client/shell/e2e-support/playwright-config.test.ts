// @vitest-environment node
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { NetworkInterfaceInfo } from "node:os";
import type { PlaywrightTestConfig } from "@playwright/test";
import { readFileSync, renameSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { repoFiles } from "../../../../scripts/lib/gate-corpus.mjs";
import { makeRepo } from "../../../../scripts/lib/temp-repo.mjs";
import {
  ANY_HTTPS_NAMED,
  ANY_HTTPS_SPEC,
  HTTPS_SPECS,
  LAN_SPEC,
  LONG_AUDIO_PLAYTHROUGH_SPEC,
  LONG_AUDIO_SPEC,
  MEDIA_RANGE_SPEC,
  ONBOARDING_SPEC,
  ROUTE_PREFIX_SPEC,
  UPGRADE_CHROMIUM_SPEC,
  UPGRADE_SPEC,
  WEBM_DECODE_SPEC,
} from "./spec-matchers";

// Addresses are from the RFC 5737 documentation range 192.0.2.0/24.

const REPO_ROOT = fileURLToPath(new URL("../../../..", import.meta.url));

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
  it("keeps the loopback, webm-decode, media-range-loopback, lan-plain-http, route-prefix and onboarding projects and drops every HTTPS project on a host with no LAN address, warning loudly", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const config = await loadConfig(LOOPBACK_ONLY, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    expect(names(config)).toEqual([
      "loopback",
      "webm-decode-chromium",
      "webm-decode-firefox",
      "media-range-loopback",
      "lan-plain-http",
      "route-prefix",
      "onboarding",
    ]);
    expect(config.webServer).toHaveLength(5);
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
    expect(all.slice(0, 7)).toEqual([
      "loopback",
      "webm-decode-chromium",
      "webm-decode-firefox",
      "media-range-loopback",
      "lan-plain-http",
      "route-prefix",
      "onboarding",
    ]);
    expect(all).toContain("https-chromium");
    expect(all).toContain("long-audio-https-chromium");
    expect(all).toContain("long-audio-playthrough-http-chromium");
    expect(all.some((n) => n.startsWith("https-upgrade"))).toBe(true);
    expect(config.webServer).toHaveLength(7);
  });

  it("gives the media-range project its own server with the short egress window, and only the long-audio server shares that window", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const project = (config.projects ?? []).find((p) => p.name === "media-range-loopback");
    const baseURL = String(project?.use?.baseURL);
    const servers = [config.webServer ?? []].flat();
    const windowed = servers.filter((s) => s.env?.SHADOWCAT_EGRESS_WINDOW_SECS !== undefined);
    expect(windowed).toHaveLength(2);
    for (const server of windowed) expect(server.env?.SHADOWCAT_EGRESS_WINDOW_SECS).toBe("3");
    // The project targets exactly one windowed server's bind address, and no other project targets it.
    const own = windowed.filter((s) => baseURL === `http://${String(s.env?.SHADOWCAT_BIND)}`);
    expect(own).toHaveLength(1);
    for (const other of (config.projects ?? []).filter((p) => p.name !== "media-range-loopback")) {
      expect(String(other.use?.baseURL ?? "")).not.toBe(baseURL);
    }
  });

  it("pipes the standard output of exactly the servers that run the short egress window, so the server's cut line reaches the test output", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const servers = [config.webServer ?? []].flat();
    const windowed = servers.filter((s) => s.env?.SHADOWCAT_EGRESS_WINDOW_SECS !== undefined);
    expect(windowed.length).toBeGreaterThan(0);
    for (const server of windowed) expect(server.stdout, String(server.env?.SHADOWCAT_BIND)).toBe("pipe");
    for (const server of servers.filter((s) => !windowed.includes(s))) {
      expect(server.stdout, String(server.env?.SHADOWCAT_BIND)).toBeUndefined();
    }
  });

  it("binds the HTTPS and long-audio servers to the LAN address, never to every interface", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const binds = [config.webServer ?? []].flat().map((s) => String(s.env?.SHADOWCAT_BIND));
    const portsOf = (prefix: string) => binds.filter((b) => b.startsWith(prefix)).length;
    expect(portsOf("192.0.2.20:")).toBe(2);
    expect(portsOf("0.0.0.0:")).toBe(1);
  });

  it("points every long-audio project at the long-audio server's port and no other project at it", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const server = [config.webServer ?? []]
      .flat()
      .find((s) => s.env?.SHADOWCAT_BIND?.startsWith("192.0.2.20:") && s.env?.SHADOWCAT_EGRESS_WINDOW_SECS !== undefined);
    expect(server, "the long-audio server is defined").toBeDefined();
    const port = String(server!.env?.SHADOWCAT_BIND).split(":")[1];
    for (const project of config.projects ?? []) {
      const onPort = new URL(String(project.use?.baseURL ?? "http://unset")).port === port;
      expect(onPort, `${project.name} ${onPort ? "targets" : "does not target"} the long-audio port`).toBe(
        String(project.name).startsWith("long-audio-"),
      );
    }
  });
});

describe("playwright.config port base", () => {
  /** Every port the config binds a server to, and every port a project's base URL names. */
  function usedPorts(config: PlaywrightTestConfig): { binds: number[]; targets: number[] } {
    const binds = [config.webServer ?? []].flat().map((s) => Number(String(s.env?.SHADOWCAT_BIND).split(":")[1]));
    const urls = [config.use?.baseURL, ...(config.projects ?? []).map((p) => p.use?.baseURL)];
    const targets = urls.filter((u): u is string => u !== undefined).map((u) => Number(new URL(u).port));
    return { binds, targets };
  }

  it("binds exactly the historical ports when the base is unset", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined, SHADOWCAT_E2E_PORT_BASE: undefined });
    expect(usedPorts(config).binds.sort()).toEqual([31999, 32000, 32001, 32002, 32003, 32004, 32005]);
  });

  it("moves every server bind and every project target together, leaving none on a historical port", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined, SHADOWCAT_E2E_PORT_BASE: "40000" });
    const { binds, targets } = usedPorts(config);
    expect(binds.sort()).toEqual([39999, 40000, 40001, 40002, 40003, 40004, 40005]);
    for (const port of targets) expect(binds, `a project targets port ${port} with no server`).toContain(port);
    expect(new Set(targets).size).toBe(7);
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
    expect([...jobs.keys()].sort(), "every e2e job is read from ci.yml").toEqual([
      "e2e-long-audio",
      "e2e-long-audio-playthrough-h1",
      "e2e-long-audio-playthrough-h2",
      "ui-e2e",
      "ui-e2e-https",
    ]);
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

describe("launch switches per engine", () => {
  /** The `args` a project launches its browser with: its own `launchOptions` replace the top-level ones. */
  function launchArgs(config: PlaywrightTestConfig, project: NonNullable<PlaywrightTestConfig["projects"]>[number]): string[] {
    return (project.use?.launchOptions ?? config.use?.launchOptions)?.args ?? [];
  }

  /** The engine a project drives; Playwright defaults an unset `browserName` to Chromium. */
  function engine(config: PlaywrightTestConfig, project: NonNullable<PlaywrightTestConfig["projects"]>[number]): string {
    return project.use?.browserName ?? config.use?.browserName ?? "chromium";
  }

  it("passes command-line switches only to Chromium projects, because WebKit exits on an unknown one", async () => {
    const loaded = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    for (const project of loaded.projects ?? []) {
      const args = launchArgs(loaded, project);
      if (engine(loaded, project) === "chromium") continue;
      expect(args, `${project.name} (${engine(loaded, project)}) must receive no Chromium switches`).toEqual([]);
    }
  });

  it("gives the Chromium projects the base switches, so the check above sees real arguments (positive control)", async () => {
    const loaded = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    const chromium = (loaded.projects ?? []).filter((p) => engine(loaded, p) === "chromium");
    expect(chromium.length).toBeGreaterThan(5);
    for (const project of chromium) {
      expect(launchArgs(loaded, project), `${project.name} lacks --use-gl=angle`).toContain("--use-gl=angle");
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

/**
 * The end-to-end test files under `e2e/`, recursively, as paths relative to the shell package.
 * The list is the repository corpus (tracked plus untracked, non-ignored files), so a new spec is
 * partition-checked before it is added to git.
 */
function listSpecs(repoRoot: string = REPO_ROOT): string[] {
  const shellRoot = "src/client/shell/";
  return repoFiles({ cwd: repoRoot, roots: [`${shellRoot}e2e`], exts: [".spec.ts"], log: () => {} }).map((p) =>
    p.slice(shellRoot.length),
  );
}

/** The projects whose `testMatch` is exactly `matcher`. */
function projectsMatching(config: PlaywrightTestConfig, matcher: RegExp): string[] {
  // Compared by source: the config module is re-imported per test, so its regexes are fresh objects.
  return (config.projects ?? [])
    .filter((p) => p.testMatch instanceof RegExp && p.testMatch.source === matcher.source)
    .map((p) => p.name ?? "");
}

describe("listSpecs reads the shared corpus", () => {
  const spec = "src/client/shell/e2e";
  const scratch = (): string =>
    makeRepo(
      {
        ".gitignore": `${spec}/ignored.spec.ts\n`,
        [`${spec}/tracked.spec.ts`]: "export {};\n",
        [`${spec}/stray.spec.ts`]: "export {};\n",
        [`${spec}/ignored.spec.ts`]: "export {};\n",
        [`${spec}/gone.spec.ts`]: "export {};\n",
      },
      [".gitignore", `${spec}/tracked.spec.ts`, `${spec}/gone.spec.ts`],
    );

  it("includes an untracked spec and omits an ignored one", () => {
    expect(listSpecs(scratch())).toEqual(["e2e/gone.spec.ts", "e2e/stray.spec.ts", "e2e/tracked.spec.ts"]);
  });

  it("omits a tracked spec deleted in the working tree", () => {
    const dir = scratch();
    renameSync(join(dir, ...spec.split("/"), "gone.spec.ts"), join(dir, "gone-aside.txt"));
    expect(listSpecs(dir)).toEqual(["e2e/stray.spec.ts", "e2e/tracked.spec.ts"]);
  });
});

/** The rule that keeps a spec out of the loopback project by mistake: a file named `https-*` must
 * match EXACTLY one of the three https project selectors, and any other file none of them.
 * @param spec A path under `e2e/`.
 * @returns A description of the violation, or `undefined` when the rule holds.
 * @example
 * ```
 * httpsSelectorViolation("e2e/https-trust.spec.ts"); // => undefined
 * ```
 */
function httpsSelectorViolation(spec: string): string | undefined {
  const selectors = [HTTPS_SPECS, UPGRADE_SPEC, UPGRADE_CHROMIUM_SPEC].filter((m) => m.test(spec)).length;
  const expected = ANY_HTTPS_NAMED.test(spec) ? 1 : 0;
  return selectors === expected ? undefined : `${spec} matches ${selectors} https selector(s), expected ${expected}`;
}

describe("spec-to-project assignment", () => {
  // Listed when the suite runs, not when the file is collected: a listing that goes wrong (slow or
  // failing) then fails these tests, and cannot stall the collection of the file's other suites.
  let specs: string[] = [];
  beforeAll(() => {
    specs = listSpecs();
  });

  it("finds the e2e specs, nested ones included", () => {
    expect(specs.length).toBeGreaterThan(10);
  });

  it("gives every https-named spec exactly one https project selector", () => {
    for (const spec of specs) {
      expect(httpsSelectorViolation(spec), spec).toBeUndefined();
    }
  });

  it("flags an https-named spec that no project selector claims (the detector fires)", () => {
    expect(httpsSelectorViolation("e2e/https-upgrade-other.spec.ts")).toContain("matches 0");
    expect(ANY_HTTPS_SPEC.some((m) => m.test("e2e/https-upgrade-other.spec.ts"))).toBe(true);
  });

  it("runs every spec in EXACTLY the project set its matcher derives: none twice, none nowhere", async () => {
    const config = await loadConfig(WITH_LAN, { CI: undefined, SHADOWCAT_E2E_LAN_IP: undefined });
    for (const spec of specs) {
      let expected: string[];
      if (LAN_SPEC.test(spec)) expected = ["lan-plain-http"];
      else if (MEDIA_RANGE_SPEC.test(spec)) expected = ["media-range-loopback"];
      else if (ROUTE_PREFIX_SPEC.test(spec)) expected = ["route-prefix"];
      else if (ONBOARDING_SPEC.test(spec)) expected = ["onboarding"];
      else if (WEBM_DECODE_SPEC.test(spec)) expected = projectsMatching(config, WEBM_DECODE_SPEC);
      else if (LONG_AUDIO_PLAYTHROUGH_SPEC.test(spec)) expected = projectsMatching(config, LONG_AUDIO_PLAYTHROUGH_SPEC);
      else if (LONG_AUDIO_SPEC.test(spec)) expected = projectsMatching(config, LONG_AUDIO_SPEC);
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
      const families = new Set(
        runners.map((n) => {
          if (n.startsWith("https-")) return "https";
          if (n.startsWith("webm-decode-")) return "webm-decode";
          if (n.startsWith("long-audio-playthrough-")) return "long-audio-playthrough";
          if (n.startsWith("long-audio-")) return "long-audio";
          return n;
        }),
      );
      expect(families.size, `${spec} runs in ${runners.join(", ")}`).toBe(1);
    }
  });
});
