// @vitest-environment node
import { describe, expect, it, vi } from "vitest";
import type { Browser } from "@playwright/test";
import { accessibilityArgs, launchWindowedChromium, PINNED_LOCALE, sandboxArgs, type BrowserProcess, type LaunchDeps } from "./windowed-chromium";

const BROWSER = { tag: "browser" } as unknown as Browser;

/** Fake dependencies: a process that records being killed, and a port that comes up after
 * `upAfter` connection attempts (never when `Infinity`). */
function fakes(upAfter: number) {
  const child: BrowserProcess & { killed: number; started: { file: string; args: string[] } | undefined } = {
    pid: 4242,
    killed: 0,
    started: undefined,
    kill() {
      this.killed += 1;
    },
  };
  let attempts = 0;
  const deps: LaunchDeps = {
    executablePath: "chromium-under-test",
    spawn: (file, args) => {
      child.started = { file, args };
      return child;
    },
    connect: async () => (++attempts > upAfter ? BROWSER : undefined),
    freePort: async () => 9999,
    prepareProfile: vi.fn(),
    platform: "linux",
    ci: true,
    timeoutMs: 120,
    pollMs: 5,
  };
  return { child, deps, attempts: () => attempts };
}

describe("launchWindowedChromium", () => {
  it("pins the UI locale, opens the debugging port on the probed port and seeds the profile", async () => {
    const { child, deps } = fakes(0);
    const launched = await launchWindowedChromium("profile-dir", ["--x=1"], deps);
    expect(launched.browser).toBe(BROWSER);
    expect(launched.child).toBe(child);
    expect(child.started?.file).toBe("chromium-under-test");
    const args = child.started!.args;
    expect(args).toContain(`--lang=${PINNED_LOCALE}`);
    expect(args).toContain("--remote-debugging-port=9999");
    expect(args).toContain("--user-data-dir=profile-dir");
    expect(args).toContain("--x=1");
    expect(args).toContain("--force-renderer-accessibility");
    expect(args.filter((a) => a === "--no-sandbox"), "the sandbox flag appears exactly once").toHaveLength(1);
    expect(args.at(-1)).toBe("about:blank");
    expect(deps.prepareProfile).toHaveBeenCalledWith("profile-dir");
    expect(child.killed, "a successful launch keeps the browser").toBe(0);
  });

  it("keeps trying until the port comes up", async () => {
    const { child, deps, attempts } = fakes(3);
    await launchWindowedChromium("p", [], deps);
    expect(attempts()).toBe(4);
    expect(child.killed).toBe(0);
  });

  it("kills the browser it started when the debugging port never comes up, then reports why", async () => {
    const { child, deps } = fakes(Infinity);
    await expect(launchWindowedChromium("p", [], deps)).rejects.toThrow("debugging port never came up");
    expect(child.killed, "an abandoned launch left its browser running").toBe(1);
  });

  it("kills the browser when connecting itself throws", async () => {
    const { child, deps } = fakes(0);
    deps.connect = async () => {
      throw new Error("boom");
    };
    await expect(launchWindowedChromium("p", [], deps)).rejects.toThrow("boom");
    expect(child.killed).toBe(1);
  });
});

describe("sandboxArgs", () => {
  it("turns the sandbox off only for Linux on CI", () => {
    expect(sandboxArgs("linux", true)).toEqual(["--no-sandbox"]);
    expect(sandboxArgs("linux", false)).toEqual([]);
    expect(sandboxArgs("darwin", true)).toEqual([]);
    expect(sandboxArgs("win32", true)).toEqual([]);
  });
});

describe("accessibilityArgs", () => {
  it("asks macOS and Linux browsers to expose their UI and leaves Windows alone", () => {
    expect(accessibilityArgs("linux")).toEqual(["--force-renderer-accessibility"]);
    expect(accessibilityArgs("darwin")).toEqual(["--force-renderer-accessibility"]);
    expect(accessibilityArgs("win32")).toEqual([]);
  });
});
