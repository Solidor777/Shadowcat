import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

/** Exit status every `invoke-button.*` script returns when the browser has no button of that name
 * (yet); any other non-zero status is a failure of the driver itself. */
export const NOT_FOUND_STATUS = 3;

/** Longest one driver invocation may take. A cold PowerShell plus UI Automation, a cold `osascript`
 * or a Python AT-SPI walk takes a few seconds; a hung accessibility provider would otherwise block
 * the test's own poll, which cannot interrupt a synchronous call. Chosen, not measured: well above
 * a cold start. The caller's poll budget is at least twice this, so one hung invocation cannot
 * consume it. */
export const INVOKE_TIMEOUT_MS = 15_000;

/** Runs a program to completion within `timeoutMs`; throws an error carrying `status` when it exits
 * non-zero, and an error with `code: "ETIMEDOUT"` (the child killed) when it outlives the timeout. */
export type RunProgram = (file: string, args: string[], timeoutMs: number) => void;

/** Runs `file` with `args`, discarding its output.
 * @param file The program.
 * @param args Its arguments.
 * @param timeoutMs Longest the program may run before it is killed.
 * @example
 * ```ts
 * runProgram(process.execPath, ["-e", ""], 5_000);
 * ```
 */
export const runProgram: RunProgram = (file, args, timeoutMs) => {
  execFileSync(file, args, { stdio: "pipe", timeout: timeoutMs });
};

/** A program and its arguments. */
export interface Invocation {
  /** The program to run. */
  file: string;
  /** Its arguments. */
  args: string[];
}

/** Exit details `execFileSync` attaches to the error it throws. */
interface ExitFailure {
  /** The program's exit status, when it ran and exited non-zero. */
  status?: number;
}

/** Path of a driver script that sits beside this module.
 * @param name The script's file name.
 * @returns Its absolute path.
 * @example
 * ```ts
 * script("invoke-button.py"); // => ".../e2e-support/invoke-button.py"
 * ```
 */
function script(name: string): string {
  return fileURLToPath(new URL(`./${name}`, import.meta.url));
}

/**
 * The command that invokes the button named `name` in the windows of process `pid` through the
 * platform's accessibility API, the only route to browser-owned dialogs (they are not web
 * content, so DOM, the DevTools accessibility tree, screenshots and DevTools input never see them).
 *   - Windows: UI Automation `InvokePattern` (`invoke-button.ps1`).
 *   - macOS: System Events accessibility through JavaScript for Automation (`invoke-button.jxa`).
 *   - Linux: AT-SPI through PyGObject (`invoke-button.py`); needs a session bus with the
 *     accessibility bus running and the browser started with accessibility enabled.
 * Every driver acts on that process's own element tree only (never global keyboard or mouse input),
 * so none can reach another application.
 * @param platform The operating system (`process.platform`).
 * @param pid The process id of the browser that owns the dialog.
 * @param name The button's accessible name.
 * @returns The command to run.
 * @throws When the platform has no driver.
 * @example
 * ```ts
 * const { file } = invocationFor("linux", 4242, "Continue to site");
 * // file === "python3"
 * ```
 */
export function invocationFor(platform: NodeJS.Platform, pid: number, name: string): Invocation {
  switch (platform) {
    case "win32":
      return {
        file: "powershell.exe",
        args: ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script("invoke-button.ps1"), "-ProcId", String(pid), "-ButtonName", name],
      };
    case "darwin":
      return { file: "osascript", args: ["-l", "JavaScript", script("invoke-button.jxa"), String(pid), name] };
    case "linux":
      return { file: "python3", args: [script("invoke-button.py"), String(pid), name] };
    default:
      throw new Error(`no browser-UI driver for platform ${platform}`);
  }
}

/**
 * Invokes the button named `name` in browser process `pid`'s windows (see `invocationFor`).
 * @param pid The process id of the browser that owns the dialog.
 * @param name The button's accessible name.
 * @param run Runs a program; replaceable in tests.
 * @param platform The operating system; replaceable in tests.
 * @returns Whether the button was found and invoked (`false`: not shown yet).
 * @throws When the driver fails for any reason other than the button being absent, including
 *   taking longer than `INVOKE_TIMEOUT_MS`.
 * @example
 * ```ts
 * const clicked = invokeBrowserButton(4242, "Continue to site", () => {}, "win32");
 * // clicked === true
 * ```
 */
export function invokeBrowserButton(
  pid: number,
  name: string,
  run: RunProgram = runProgram,
  platform: NodeJS.Platform = process.platform,
): boolean {
  const { file, args } = invocationFor(platform, pid, name);
  try {
    run(file, args, INVOKE_TIMEOUT_MS);
    return true;
  } catch (e) {
    if ((e as ExitFailure).status === NOT_FOUND_STATUS) return false;
    throw e;
  }
}
