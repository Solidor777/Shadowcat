// @vitest-environment node
import { describe, expect, it } from "vitest";
import { INVOKE_TIMEOUT_MS, invocationFor, invokeBrowserButton, runProgram, type RunProgram } from "./browser-ui";

describe("invocationFor", () => {
  it("drives Windows UI Automation through PowerShell with the process and button", () => {
    const { file, args } = invocationFor("win32", 4242, "Continue to site");
    expect(file).toBe("powershell.exe");
    expect(args).toEqual(expect.arrayContaining(["-ProcId", "4242", "-ButtonName", "Continue to site"]));
    expect(args.find((a) => a.endsWith("invoke-button.ps1"))).toBeDefined();
  });

  it("drives macOS System Events through JavaScript for Automation with the process and button", () => {
    const { file, args } = invocationFor("darwin", 4242, "Continue to site");
    expect(file).toBe("osascript");
    expect(args.slice(0, 2)).toEqual(["-l", "JavaScript"]);
    expect(args.find((a) => a.endsWith("invoke-button.jxa"))).toBeDefined();
    expect(args.slice(-2)).toEqual(["4242", "Continue to site"]);
  });

  it("drives Linux AT-SPI through Python with the process and button", () => {
    const { file, args } = invocationFor("linux", 4242, "Continue to site");
    expect(file).toBe("python3");
    expect(args.find((a) => a.endsWith("invoke-button.py"))).toBeDefined();
    expect(args.slice(-2)).toEqual(["4242", "Continue to site"]);
  });

  it("names a platform with no driver instead of guessing one", () => {
    expect(() => invocationFor("freebsd", 1, "x")).toThrow("no browser-UI driver for platform freebsd");
  });
});

describe("invokeBrowserButton", () => {
  it("runs the platform's command and reports the button invoked", () => {
    const calls: { file: string; timeoutMs: number }[] = [];
    const run: RunProgram = (file, _args, timeoutMs) => void calls.push({ file, timeoutMs });
    expect(invokeBrowserButton(4242, "Continue to site", run, "linux")).toBe(true);
    expect(calls).toEqual([{ file: "python3", timeoutMs: INVOKE_TIMEOUT_MS }]);
  });

  it("reports false, not an error, when the browser has no such button yet", () => {
    const run: RunProgram = () => {
      throw Object.assign(new Error("exit 3"), { status: 3 });
    };
    for (const platform of ["win32", "darwin", "linux"] as const) {
      expect(invokeBrowserButton(1, "x", run, platform), platform).toBe(false);
    }
  });

  it("rethrows any other failure: a broken driver is not 'not shown yet'", () => {
    const run: RunProgram = () => {
      throw Object.assign(new Error("exit 1"), { status: 1 });
    };
    expect(() => invokeBrowserButton(1, "x", run, "linux")).toThrow("exit 1");
  });

  it("rethrows a timeout instead of reading it as 'not shown yet'", () => {
    const run: RunProgram = () => {
      throw Object.assign(new Error("spawnSync ETIMEDOUT"), { code: "ETIMEDOUT" });
    };
    expect(() => invokeBrowserButton(1, "x", run, "win32")).toThrow("ETIMEDOUT");
  });

  it("bounds every driver run, because a hung accessibility call cannot be interrupted from the poll", () => {
    let seen: number | undefined;
    invokeBrowserButton(1, "x", (_f, _a, timeoutMs) => void (seen = timeoutMs), "win32");
    expect(seen).toBe(INVOKE_TIMEOUT_MS);
    expect(INVOKE_TIMEOUT_MS).toBeGreaterThan(0);
  });
});

describe("runProgram", () => {
  it("kills a program that outlives its timeout and throws ETIMEDOUT", () => {
    const started = Date.now();
    let error: unknown;
    try {
      runProgram(process.execPath, ["-e", "setTimeout(() => {}, 20000)"], 300);
    } catch (e) {
      error = e;
    }
    expect((error as { code?: string } | undefined)?.code).toBe("ETIMEDOUT");
    expect(Date.now() - started).toBeLessThan(10_000);
  });

  it("returns once a program that finishes inside its timeout exits 0", () => {
    expect(() => runProgram(process.execPath, ["-e", ""], 20_000)).not.toThrow();
  });

  it("carries the exit status of a failing program", () => {
    let error: unknown;
    try {
      runProgram(process.execPath, ["-e", "process.exit(3)"], 20_000);
    } catch (e) {
      error = e;
    }
    expect((error as { status?: number } | undefined)?.status).toBe(3);
  });
});
