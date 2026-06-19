// Spawns the Rust test_server (debug build), parses its printed bind address and
// e2e fixture, and exposes login + teardown. Node-only; used by *.e2e.test.ts,
// which run in the dedicated e2e CI job (the default unit run excludes them).
import { spawn, type ChildProcess } from "node:child_process";
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
  const proc: ChildProcess = spawn(
    "cargo",
    ["run", "-q", "-p", "shadowcat", "--bin", "test_server"],
    {
      cwd: repoRoot,
      stdio: ["ignore", "pipe", "inherit"],
      // Windows needs the shell to resolve `cargo` via PATHEXT; POSIX runs the
      // process in its own group so the whole tree (cargo -> test_server) can be
      // signalled on teardown.
      shell: isWindows,
      detached: !isWindows,
    },
  );

  // `cargo run` spawns test_server as a child, so killing `proc` alone orphans
  // the server (holding its port). Kill the whole process tree.
  const stop = (): void => {
    if (proc.pid === undefined) return;
    if (isWindows) {
      spawn("taskkill", ["/pid", String(proc.pid), "/T", "/F"], { stdio: "ignore" });
    } else {
      try {
        process.kill(-proc.pid, "SIGTERM");
      } catch {
        proc.kill("SIGTERM");
      }
    }
  };

  let baseUrl = "";
  let fixture: Fixture | null = null;
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("test_server did not start within 220s")),
      220_000,
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
