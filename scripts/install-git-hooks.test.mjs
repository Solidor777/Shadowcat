import { test, expect } from "vitest";
import { hooksDir, shouldSkip, configPlan, installHooks, HOOK_FILES } from "./install-git-hooks.mjs";

test("the hooks directory is resolved under the repo root with the platform separator", () => {
  const d = hooksDir("/repo");
  expect(d.replace(/\\/g, "/")).toBe("/repo/scripts/git-hooks");
});

test("installation is skipped under CI", () => {
  expect(shouldSkip({ CI: "true" })).toBe(true);
  expect(shouldSkip({ GITHUB_ACTIONS: "true" })).toBe(true);
  expect(shouldSkip({})).toBe(false);
});

test("configPlan enables worktree config before writing a worktree-scoped hooksPath", () => {
  const plan = configPlan("/repo");
  expect(plan[0].args).toEqual(["config", "extensions.worktreeConfig", "true"]);
  expect(plan[1].args[0]).toBe("config");
  expect(plan[1].args[1]).toBe("--worktree");
  expect(plan[1].args[2]).toBe("core.hooksPath");
  expect(plan[1].args[3].replace(/\\/g, "/")).toBe("/repo/scripts/git-hooks");
});

test("installHooks reports unarmed rather than throwing when git itself fails", () => {
  const execFile = () => {
    throw new Error("spawn git ENOENT");
  };
  const result = installHooks({ execFile });
  expect(result.armed).toBe(false);
  expect(result.reason).toMatch(/ENOENT/);
});

test("installHooks reports unarmed, not an error exit, when the hooks directory is missing", () => {
  const execFile = (_cmd, args) => (args[0] === "rev-parse" ? "/repo\n" : "");
  const exists = () => false;
  const result = installHooks({ execFile, exists });
  expect(result.armed).toBe(false);
  expect(result.reason).toMatch(/missing/);
});

test("installHooks reports unarmed when a config write throws (e.g. an unwritable .git/config)", () => {
  const execFile = (_cmd, args) => {
    if (args[0] === "rev-parse") return "/repo\n";
    throw new Error("permission denied");
  };
  const exists = () => true;
  const result = installHooks({ execFile, exists });
  expect(result.armed).toBe(false);
  expect(result.reason).toMatch(/permission denied/);
});

test("installHooks chmods exactly the known hook files, not every directory entry", () => {
  const execFile = (_cmd, args) => (args[0] === "rev-parse" ? "/repo\n" : "");
  const exists = () => true;
  const chmodded = [];
  const result = installHooks({ execFile, exists, chmod: (p) => chmodded.push(p) });
  expect(result.armed).toBe(true);
  expect(chmodded.map((p) => p.replace(/\\/g, "/"))).toEqual(
    HOOK_FILES.map((name) => `/repo/scripts/git-hooks/${name}`),
  );
});

test("installHooks skips chmod for a hook file that does not exist without failing the install", () => {
  const execFile = (_cmd, args) => (args[0] === "rev-parse" ? "/repo\n" : "");
  const exists = (p) => !p.replace(/\\/g, "/").endsWith("pre-push");
  const chmodded = [];
  const result = installHooks({ execFile, exists, chmod: (p) => chmodded.push(p) });
  expect(result.armed).toBe(true);
  expect(chmodded.map((p) => p.replace(/\\/g, "/"))).toEqual(["/repo/scripts/git-hooks/pre-commit"]);
});
