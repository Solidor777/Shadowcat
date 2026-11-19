import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import type { AddressInfo } from "node:net";
import { join } from "node:path";
import type { Browser } from "@playwright/test";

/** The UI locale every windowed Chromium is pinned to. The UI drivers find the interstitial's button
 * by its English accessible name ("Continue to site"), so the locale must not follow the host's.
 * Measured on Windows only: under `--lang=fr` the button is named "Accéder au site". */
export const PINNED_LOCALE = "en-US";

/** Launch arguments that make the browser expose its UI to the platform's accessibility API, which
 * the UI drivers read. Windows UI Automation needs none (measured); macOS and Linux build the tree
 * on demand and are asked to keep it on. Not yet measured on those two platforms.
 * @param platform The operating system (`process.platform`).
 * @returns The arguments.
 * @example
 * ```
 * accessibilityArgs("linux"); // => ["--force-renderer-accessibility"]
 * ```
 */
export function accessibilityArgs(platform: NodeJS.Platform): string[] {
  return platform === "linux" || platform === "darwin" ? ["--force-renderer-accessibility"] : [];
}

/** Launch arguments for the process sandbox. Chromium's Linux sandbox needs unprivileged user
 * namespaces, which a CI runner may restrict, so the sandbox is turned off only on a Linux CI run;
 * a developer run keeps it.
 * @param platform The operating system (`process.platform`).
 * @param ci Whether the run is on CI.
 * @returns The arguments.
 * @example
 * ```
 * sandboxArgs("linux", true); // => ["--no-sandbox"]
 * ```
 */
export function sandboxArgs(platform: NodeJS.Platform, ci: boolean): string[] {
  return platform === "linux" && ci ? ["--no-sandbox"] : [];
}

/** How long the browser's debugging port may take to come up. */
const LAUNCH_TIMEOUT_MS = 30_000;

/** Gap between connection attempts. */
const POLL_INTERVAL_MS = 250;

/** The part of a spawned browser process the launcher and its caller use. */
export interface BrowserProcess {
  /** The process id UI Automation addresses. */
  pid?: number;
  /** Ends the process. */
  kill: () => void;
}

/** A running windowed browser and its process. */
export interface WindowedChromium {
  /** The DevTools connection. */
  browser: Browser;
  /** The browser process. */
  child: BrowserProcess;
}

/** What the launcher needs from the outside; replaceable in tests. */
export interface LaunchDeps {
  /** The browser executable. */
  executablePath: string;
  /** Starts the browser. */
  spawn: (file: string, args: string[]) => BrowserProcess;
  /** Connects to the debugging port; `undefined` while it is not up yet. */
  connect: (port: number) => Promise<Browser | undefined>;
  /** A free loopback port. */
  freePort: () => Promise<number>;
  /** Seeds the profile so HTTPS-First Mode is on. */
  prepareProfile: (profile: string) => void;
  /** The operating system, which decides the accessibility arguments. */
  platform: NodeJS.Platform;
  /** Whether the run is on CI, which decides the sandbox arguments. */
  ci: boolean;
  /** How long to wait for the debugging port. */
  timeoutMs: number;
  /** Gap between connection attempts. */
  pollMs: number;
}

/** The real dependencies: Playwright's Chromium, a real process and a real loopback port.
 * @returns The dependencies.
 * @example
 * ```
 * const deps = await defaultLaunchDeps();
 * ```
 */
export async function defaultLaunchDeps(): Promise<LaunchDeps> {
  const { chromium } = await import("@playwright/test");
  return {
    executablePath: chromium.executablePath(),
    spawn: (file, args) => spawn(file, args, { stdio: "ignore" }),
    connect: (port) => chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => undefined),
    freePort: () =>
      new Promise<number>((resolve, reject) => {
        const probe = createServer();
        probe.once("error", reject);
        probe.listen(0, "127.0.0.1", () => {
          const { port } = probe.address() as AddressInfo;
          probe.close(() => resolve(port));
        });
      }),
    prepareProfile: (profile) => {
      mkdirSync(join(profile, "Default"), { recursive: true });
      writeFileSync(join(profile, "Default", "Preferences"), JSON.stringify({ https_only_mode_enabled: true }));
    },
    platform: process.platform,
    ci: process.env.CI !== undefined,
    timeoutMs: LAUNCH_TIMEOUT_MS,
    pollMs: POLL_INTERVAL_MS,
  };
}

/**
 * Starts a headed Chromium with HTTPS-First Mode on in its profile, pinned to `PINNED_LOCALE`, and
 * connects to it over DevTools. Headed and self-launched because the interstitial is browser UI
 * that only a real window carries, and HTTPS-First is a profile preference, not a launch switch.
 *
 * INVARIANT: the browser process never outlives a failed launch. A process started here and then
 * abandoned by a timeout would keep its window and its profile open for the rest of the run.
 * @param profile The user-data directory.
 * @param extraArgs Further launch arguments.
 * @param deps The outside world; defaults to the real one.
 * @returns The connection and the browser process.
 * @throws When the debugging port does not come up in time (the process is killed first).
 * @example
 * ```
 * const { browser, child } = await launchWindowedChromium("out/profile", []);
 * ```
 */
export async function launchWindowedChromium(
  profile: string,
  extraArgs: string[],
  deps?: LaunchDeps,
): Promise<WindowedChromium> {
  const d = deps ?? (await defaultLaunchDeps());
  d.prepareProfile(profile);
  const port = await d.freePort();
  const child = d.spawn(d.executablePath, [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    `--lang=${PINNED_LOCALE}`,
    "--no-first-run",
    "--no-default-browser-check",
    ...accessibilityArgs(d.platform),
    ...sandboxArgs(d.platform, d.ci),
    ...extraArgs,
    "about:blank",
  ]);
  try {
    const deadline = Date.now() + d.timeoutMs;
    for (;;) {
      const browser = await d.connect(port);
      if (browser !== undefined) return { browser, child };
      if (Date.now() >= deadline) throw new Error("the browser's debugging port never came up");
      await new Promise((resolve) => setTimeout(resolve, d.pollMs));
    }
  } catch (e) {
    child.kill();
    throw e;
  }
}
