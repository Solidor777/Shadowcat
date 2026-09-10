import { test, expect } from "vitest";
import { classify } from "../.claude/hooks/guard-git.mjs";

const armed = { hooksPathSet: true };

test("--no-verify is denied on commit and on push", () => {
  expect(classify("git commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git push --no-verify origin main", armed).deny).toBe(true);
});

test("commit -n is denied, and -m text containing -n is not mistaken for it", () => {
  expect(classify("git commit -n -m x", armed).deny).toBe(true);
  expect(classify('git commit -am "fix -n handling in the parser"', armed).deny).toBe(false);
  expect(classify('git commit -m "a -n b"', armed).deny).toBe(false);
});

test("a combined short flag carrying n is denied", () => {
  expect(classify("git commit -an -m x", armed).deny).toBe(true);
});

test("an inline hooksPath override is denied", () => {
  expect(classify("git -c core.hooksPath=/dev/null commit -m x", armed).deny).toBe(true);
});

test("mutating the hooks configuration is denied", () => {
  expect(classify("git config --worktree core.hooksPath /tmp/x", armed).deny).toBe(true);
  expect(classify("git config --unset core.hooksPath", armed).deny).toBe(true);
  expect(classify("git config extensions.worktreeConfig false", armed).deny).toBe(true);
});

test("any commit or push in an unarmed repository is denied", () => {
  const bare = { hooksPathSet: false };
  expect(classify("git commit -m x", bare).deny).toBe(true);
  expect(classify("git commit -m x", bare).reason).toMatch(/pnpm install/);
  expect(classify("git push origin main", bare).deny).toBe(true);
});

test("ordinary git work is untouched", () => {
  expect(classify("git commit -m x", armed).deny).toBe(false);
  expect(classify("git push origin main", armed).deny).toBe(false);
  expect(classify("git status --porcelain", armed).deny).toBe(false);
  expect(classify("git log --oneline -5", armed).deny).toBe(false);
  expect(classify("npm run build", armed).deny).toBe(false);
});

test("a bypass hidden behind a chain separator is still denied", () => {
  expect(classify("echo hi && git commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("cd /repo; git push --no-verify", armed).deny).toBe(true);
});
