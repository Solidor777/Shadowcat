import { spawn } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { createServer } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Environment shared by every test server process; each server adds its own `SHADOWCAT_BIND`. The
 * Playwright config's web servers and the servers a spec starts itself (`startServer`) read this one
 * record, so the seeded admin and the relaxed throttles are the same everywhere. */
export const E2E_SERVER_ENV: Record<string, string> = {
  SHADOWCAT_ADMIN_USER: "ops",
  SHADOWCAT_ADMIN_PASSWORD: "pw-boot",
  SHADOWCAT_SETUP_TOKEN: "off",
  SHADOWCAT_DB: "sqlite::memory:",
  SHADOWCAT_LOG: "warn",
  // The first-use tour offer is a fixed toast over the bottom-left of the viewport, where tiles and
  // pickers sit; left on, it intercepts the pointer in every spec that opens a fresh world. The
  // tours themselves stay reachable through the settings panel's replay buttons.
  SHADOWCAT_ONBOARDING_ENABLED: "false",
  // The suite logs in as the same seeded admin identity many times across
  // specs within the auth-throttle's 60s sliding window (`/api/login` allows
  // LOGIN_PER_MIN_PER_IDENTITY=10 by default) — relax the budgets so
  // the e2e login pattern itself can never trip them. The `http::throttle` module's
  // production default consts are untouched; this only overrides this webServer process.
  SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY: "10000",
  SHADOWCAT_LOGIN_PER_MIN_PER_IP: "10000",
  SHADOWCAT_INVITE_PER_MIN_PER_ACCOUNT: "10000",
  SHADOWCAT_INVITE_PER_MIN_PER_IP: "10000",
  // Same reasoning as the login/invite overrides above, for `POST /api/worlds`'s
  // per-account budget (`WORLD_CREATE_PER_MIN_PER_ACCOUNT=10`): each worker's
  // seeded identity creates far more than 10 worlds across the whole suite, and
  // `reuseExistingServer` keeps one server (and its sliding-window throttle state)
  // alive across repeated local invocations of a subset of specs.
  SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT: "10000",
};

/** Playwright's output folder (`outputDir` of the config). Playwright empties it at the start of every
 * run, before any web server starts, so everything under it never accumulates. It is git-ignored as
 * `test-results/`. */
export const E2E_OUTPUT_DIR: string = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "test-results");

/** The folder under `E2E_OUTPUT_DIR` that holds one data folder per test server. */
export const E2E_SERVER_DATA_DIR: string = path.join(E2E_OUTPUT_DIR, "server-data");

/**
 * The settings that point a test server's written folders at its own data folder.
 * `assets` and `trash` are siblings, so `Config::validate`'s rule that the trash must not nest inside
 * the assets folder holds. The logs and backups folders have no location of their own for an
 * in-memory database, and for a file-backed one they resolve beside the database file, which the
 * caller places inside the same data folder.
 * @param dataDir The server's own data folder.
 * @returns The `SHADOWCAT_ASSETS_DIR` and `SHADOWCAT_TRASH_DIR` settings.
 * @example
 * ```
 * const env = serverDataEnv("/repo/src/client/shell/test-results/server-data/loopback");
 * ```
 */
export function serverDataEnv(dataDir: string): Record<string, string> {
  return {
    SHADOWCAT_ASSETS_DIR: path.join(dataDir, "assets"),
    SHADOWCAT_TRASH_DIR: path.join(dataDir, "trash"),
  };
}

/**
 * The environment of a shared test server: `E2E_SERVER_ENV`, a data folder of its own named `name`
 * under `E2E_SERVER_DATA_DIR`, and the server's extra settings.
 * @param name The server's name; two servers never share one.
 * @param extra Settings layered last (the bind address and any per-server switches).
 * @returns The complete environment.
 * @example
 * ```
 * const env = sharedServerEnv("loopback", { SHADOWCAT_BIND: "127.0.0.1:31999" });
 * ```
 */
export function sharedServerEnv(name: string, extra: Record<string, string>): Record<string, string> {
  return { ...E2E_SERVER_ENV, ...serverDataEnv(path.join(E2E_SERVER_DATA_DIR, name)), ...extra };
}

/** How long `startServer` waits for a new server's `/health` to answer. */
const READY_TIMEOUT_MS = 60_000;

/** The `/health` poll interval while a server starts. */
const READY_POLL_MS = 200;

/**
 * Resolves the built server binary the e2e suite drives.
 * The long-audio jobs transcode 20-minute tracks at import, which a debug build does about 30 times
 * slower than a release build, so `SHADOWCAT_E2E_PROFILE=release` selects the binary
 * `e2e:build:release` produces; any other value selects the debug binary.
 * @param repoRoot The repository root the `target` directory lives under.
 * @param env The process environment to read `SHADOWCAT_E2E_PROFILE` from.
 * @param platform The platform whose executable suffix applies.
 * @returns The absolute path of the binary.
 * @example
 * ```
 * const bin = serverBinaryPath("/repo", {}, "linux"); // "/repo/target/debug/shadowcat"
 * ```
 */
export function serverBinaryPath(
  repoRoot: string,
  env: Record<string, string | undefined>,
  platform: string = process.platform,
): string {
  return path.join(
    repoRoot,
    "target",
    env.SHADOWCAT_E2E_PROFILE === "release" ? "release" : "debug",
    platform === "win32" ? "shadowcat.exe" : "shadowcat",
  );
}

/**
 * Asks the operating system for a TCP port nobody is listening on.
 * The port is released before it is returned, so another process could take it before the caller
 * binds it; a server that fails to bind surfaces as `startServer` failing to become ready.
 * @returns The free port.
 * @example
 * ```
 * const port = await freePort();
 * ```
 */
export async function freePort(): Promise<number> {
  return new Promise<number>((resolve, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const address = probe.address();
      const port = typeof address === "object" && address !== null ? address.port : 0;
      probe.close(() => resolve(port));
    });
  });
}

/** A server process a spec started itself. */
export interface RunningServer {
  /** The server's origin, e.g. `http://127.0.0.1:41234`. */
  url: string;
  /** Resolves with the process's exit code once it has exited (`null` when a signal ended it). */
  exited: Promise<number | null>;
  /** Stops the process if it is still running and resolves once it has exited. */
  stop: () => Promise<void>;
}

/** What `startServer` needs to start one server. */
export interface StartServerOptions {
  /** The repository root, used as the process's working directory and to find the binary. */
  repoRoot: string;
  /** The server's data folder: its database file, assets, trash, logs and backups all live inside it.
   * A caller places it under `E2E_OUTPUT_DIR` so Playwright clears it every run. */
  dataDir: string;
  /** Environment layered over `E2E_SERVER_ENV` (the bind address is set by `startServer`). */
  env?: Record<string, string>;
}

/**
 * The environment of a server a spec starts itself: `E2E_SERVER_ENV` over a file-backed database
 * inside `options.dataDir`, with the assets and trash folders inside it too.
 * @param options The data folder and extra environment.
 * @param port The loopback port the server binds.
 * @returns The complete environment, without the parent process's own variables.
 * @example
 * ```
 * const env = startedServerEnv({ repoRoot: "/repo", dataDir: "/repo/src/client/shell/test-results/data" }, 41234);
 * ```
 */
export function startedServerEnv(options: StartServerOptions, port: number): Record<string, string> {
  return {
    ...E2E_SERVER_ENV,
    ...serverDataEnv(options.dataDir),
    SHADOWCAT_DB: `sqlite:${path.join(options.dataDir, "shadowcat.db").replaceAll("\\", "/")}`,
    ...options.env,
    SHADOWCAT_BIND: `127.0.0.1:${port}`,
  };
}

/**
 * Starts a server process on a free loopback port and waits until its `/health` answers.
 * The process inherits no output; a spec that needs to read its logs starts it another way.
 * @param options The repository root, data folder and extra environment.
 * @returns The running server.
 * @throws When the process exits before it is ready or `/health` never answers.
 * @example
 * ```
 * const server = await startServer({ repoRoot: "/repo", dataDir: "/repo/src/client/shell/test-results/data" });
 * await server.stop();
 * ```
 */
export async function startServer(options: StartServerOptions): Promise<RunningServer> {
  const port = await freePort();
  const child: ChildProcess = spawn(serverBinaryPath(options.repoRoot, process.env), [], {
    cwd: options.repoRoot,
    stdio: "ignore",
    env: { ...process.env, ...startedServerEnv(options, port) },
  });
  const exited = new Promise<number | null>((resolve) => {
    child.once("exit", (code) => resolve(code));
  });
  let hasExited = false;
  void exited.then(() => {
    hasExited = true;
  });
  const url = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + READY_TIMEOUT_MS;
  for (;;) {
    if (hasExited) throw new Error(`the server exited before it was ready (${url})`);
    try {
      if ((await fetch(`${url}/health`)).ok) break;
    } catch {
      // The port is not accepting yet; the next poll retries.
    }
    if (Date.now() > deadline) {
      child.kill();
      throw new Error(`the server did not answer ${url}/health within ${READY_TIMEOUT_MS} ms`);
    }
    await new Promise((resolve) => setTimeout(resolve, READY_POLL_MS));
  }
  return {
    url,
    exited,
    stop: async () => {
      if (!hasExited) child.kill();
      await exited;
    },
  };
}
