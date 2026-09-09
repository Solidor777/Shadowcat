import { test, expect } from "vitest";
import { hooksDir, shouldSkip, configPlan } from "./install-git-hooks.mjs";

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
