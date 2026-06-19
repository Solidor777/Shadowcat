// Builds the Rust test_server (debug), then runs the binary DIRECTLY (not via
// `cargo run`, which would spawn it as a hard-to-reap grandchild). Parses the
// printed bind address + e2e fixture and exposes login + teardown. Node-only;
// used by *.e2e.test.ts, run in the dedicated e2e CI job (the unit run excludes
// them).
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

export interface Fixture {
  world: string;
  doc: string;
  gm: string;
  player: string;
}

export interface TestServer {
  baseUrl: string;
  wsUrl: string;
  fixture: Fixture;
  stop(): void;
}

// .../src/client/core/src/e2e/server-process.ts -> repo root is five levels up.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../../..");

export async function startTestServer(): Promise<TestServer> {
  const isWindows = process.platform === "win32";
  const exe = path.join(repoRoot, "target", "debug", isWindows ? "test_server.exe" : "test_server");

  // Build first (fast if already built; the CI job pre-builds). `shell` lets
  // Windows resolve `cargo` via PATHEXT. Building separately means the long-lived
  // process is the binary itself, not a cargo wrapper with a grandchild.
  const build = spawnSync("cargo", ["build", "-p", "shadowcat", "--bin", "test_server"], {
    cwd: repoRoot,
    stdio: "inherit",
    shell: isWindows,
  });
  if (build.status !== 0) throw new Error(`cargo build test_server failed (${build.status})`);

  const proc: ChildProcess = spawn(exe, [], {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "inherit"],
  });

  // `proc` IS the server (run directly, no cargo grandchild), so a direct kill
  // reaps it — no orphaned process holding the port. /F on Windows for certainty.
  const stop = (): void => {
    if (proc.pid === undefined) return;
    if (isWindows) {
      spawnSync("taskkill", ["/pid", String(proc.pid), "/T", "/F"], { stdio: "ignore" });
    } else {
      proc.kill("SIGKILL");
    }
  };

  let baseUrl = "";
  let fixture: Fixture | null = null;
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("test_server did not start within 20s")),
      20_000,
    );
    let buf = "";
    proc.stdout!.on("data", (chunk: Buffer) => {
      buf += chunk.toString();
      const addr = /test_server: (http:\/\/[\d.:]+)/.exec(buf);
      if (addr) baseUrl = addr[1];
      const fx = /e2e-fixture: (\{.*\})/.exec(buf);
      if (fx) fixture = JSON.parse(fx[1]) as Fixture;
      if (baseUrl && fixture) {
        clearTimeout(timer);
        resolve();
      }
    });
    proc.on("error", reject);
    proc.on("exit", (code) => reject(new Error(`test_server exited early (code ${code})`)));
  });

  // Startup is parsed; stop reading stdout and let the event loop ignore the
  // child so the test runner can exit promptly after stop() (the kill is async).
  proc.stdout?.removeAllListeners("data");
  proc.stdout?.destroy();
  proc.unref();

  const wsUrl = baseUrl.replace(/^http/, "ws") + "/ws";
  return { baseUrl, wsUrl, fixture: fixture!, stop };
}

/** Log in via /api/login; returns the session cookie header value (name=value). */
export async function login(
  baseUrl: string,
  username: string,
  password: string,
): Promise<string> {
  const res = await fetch(`${baseUrl}/api/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(`login failed: ${res.status}`);
  const cookies = res.headers.getSetCookie();
  if (cookies.length === 0) throw new Error("no session cookie returned");
  // Keep only the cookie name=value pair (drop attributes like Path/HttpOnly).
  return cookies[0].split(";")[0];
}
