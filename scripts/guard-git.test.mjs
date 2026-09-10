import { test, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { execPath } from "node:process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { classify, looksLikeCommitOrPush, bestEffortCommand } from "../.claude/hooks/guard-git.mjs";

const armed = { hooksPathSet: true };
const HOOK_PATH = resolve(dirname(fileURLToPath(import.meta.url)), "..", ".claude", "hooks", "guard-git.mjs");

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

// A bypass flag placed AFTER the message must be exactly as denied as one placed before it — a
// classifier that only scans up to `-m` proves nothing about the ordering it never looks at.
test("a bypass flag placed after the message is denied, not just before it", () => {
  expect(classify('git commit -m "fix" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "fix" -n', armed).deny).toBe(true);
  expect(classify('git commit --amend -m "x" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "fix" -n -a', armed).deny).toBe(true);
  expect(classify('git commit --no-verify -m "fix"', armed).deny).toBe(true); // control: before is still denied
});

// `-n` means `--dry-run` for push and mutates nothing; only `--no-verify` is a real bypass there.
test("-n is a bypass flag on commit but not on push", () => {
  expect(classify("git push -n origin main", armed).deny).toBe(false);
  expect(classify("git push --no-verify origin main", armed).deny).toBe(true);
});

// A quoted commit message is opaque content: a bare word inside it that happens to equal a
// sensitive config key, or prose that quotes a bypass flag, must never be read as a real one — and
// a chain-separator character quoted inside the message must never manufacture a fake segment.
test("a commit message containing the gate's own vocabulary as prose stays allowed", () => {
  expect(classify('git commit -m "explain core.hooksPath behavior in docs"', armed).deny).toBe(
    false,
  );
  expect(classify('git commit -m "reproduce the --no-verify bug"', armed).deny).toBe(false);
  expect(
    classify('git commit -m "run this; git commit --no-verify -m x" --amend', armed).deny,
  ).toBe(false);
});

// GIT_CONFIG_KEY_<n>/GIT_CONFIG_VALUE_<n>/GIT_CONFIG_COUNT (git 2.31+) redirect core.hooksPath for
// a single invocation without the string ever appearing as a `-c` token.
test("a GIT_CONFIG_* environment override around a commit is denied", () => {
  expect(
    classify(
      "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null git commit -m x",
      armed,
    ).deny,
  ).toBe(true);
  // An empty value is still a real override — the key is what redirects the gate, not the value's
  // contents.
  expect(
    classify(
      "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0= git commit -m x",
      armed,
    ).deny,
  ).toBe(true);
  // An unrelated environment variable, or a GIT_CONFIG_* mention safely confined to a quoted
  // commit message, must not trigger the same refusal.
  expect(classify("DEBUG=1 git commit -m x", armed).deny).toBe(false);
  expect(classify('git commit -m "explain GIT_CONFIG_COUNT=1 usage"', armed).deny).toBe(false);
});

// A value-taking global flag before the subcommand (`-C <path>`, `--git-dir`, `--work-tree`,
// `--namespace`) must not be mistaken for the subcommand itself, and a path-qualified or
// `.exe`-suffixed git binary must still be recognised by basename.
test("a global flag or a path-qualified git binary does not hide the subcommand from the scan", () => {
  expect(classify("git -C /tmp commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git --git-dir /tmp/.git commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("/usr/bin/git commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git.exe commit --no-verify -m x", armed).deny).toBe(true);
  // The same forms must stay allowed when there is no bypass flag.
  expect(classify("git -C /tmp commit -m x", armed).deny).toBe(false);
  expect(classify("/usr/bin/git status", armed).deny).toBe(false);
});

// `git config <key>` with no `--get` still reads; only a value argument, or an explicit write
// flag, makes it a write.
test("a bare git config read of the gate keys stays allowed; a bare write is still denied", () => {
  expect(classify("git config core.hooksPath", armed).deny).toBe(false);
  expect(classify("git config extensions.worktreeConfig", armed).deny).toBe(false);
  expect(classify("git config core.hooksPath /tmp/x", armed).deny).toBe(true);
});

// `hooksPathSet` may be a lazy accessor (the real entry point passes one); it must be invoked only
// on the path that actually needs it, never for a command that is not git at all or that denies
// for an unrelated reason first.
test("hooksPathSet is a lazy accessor invoked only when a commit/push actually reaches the armed check", () => {
  let calls = 0;
  const lazy = { hooksPathSet: () => (calls++, true) };
  classify("npm run build", lazy);
  classify("git status", lazy);
  classify("git commit --no-verify -m x", lazy); // denied before the armed check
  expect(calls).toBe(0);
  classify("git commit -m x", lazy); // nothing else denies; reaches the armed check
  expect(calls).toBe(1);
});

test("looksLikeCommitOrPush matches raw text naming a commit or push, even when not valid JSON", () => {
  expect(looksLikeCommitOrPush('{"tool_input":{"command":"git commit -m x"}}')).toBe(true);
  expect(looksLikeCommitOrPush('{"tool_input":{"command":"git push --no-verify"')).toBe(true); // truncated JSON
  expect(looksLikeCommitOrPush('{"tool_input":{"command":"git status"}}')).toBe(false);
  expect(looksLikeCommitOrPush("not json at all, no git here")).toBe(false);
});

// End-to-end: a malformed payload that still names a commit or push must fail CLOSED (deny) rather
// than open, because --no-verify and the unarmed-repository check are the only two cases this
// guard uniquely covers.
test("a malformed payload naming a commit or push fails closed at the real entry point", () => {
  const result = spawnSync(execPath, [HOOK_PATH], {
    input: '{"tool_input":{"command":"git commit --no-verify -m x"', // truncated, invalid JSON
    encoding: "utf8",
  });
  expect(result.stdout).toContain('"permissionDecision":"deny"');
});

test("a malformed payload with no git commit/push in it fails open at the real entry point", () => {
  const result = spawnSync(execPath, [HOOK_PATH], {
    input: "not json at all",
    encoding: "utf8",
  });
  expect(result.stdout.trim()).toBe("");
});

// The tokenizer implements the POSIX shell quoting grammar as a specification, not as a patch over
// the one case that exposed a gap: inside single quotes every character is literal with no
// escapes; inside double quotes a backslash escapes only " \ $ and a backtick and is otherwise
// literal; outside quotes a backslash escapes whatever follows. Every case below is derived from
// that grammar, not from a specific reported bypass, and asserts both the bypass form (denied) and
// the innocent form (allowed) where the two differ.

test("an escaped double quote inside a double-quoted message does not close the span early", () => {
  // CRITICAL regression: with naive quote-matching, \" closes the string early, the real closing
  // quote then opens a new unterminated span, and --no-verify is swallowed into one opaque token
  // that no exact-match flag test can see.
  expect(classify('git commit -m "fix: escape \\" test" --no-verify', armed).deny).toBe(true);
  // An even number of escaped quotes happened to still work under the old naive implementation;
  // asserted here as a named case of the same grammar, not an accident of parity.
  expect(classify('git commit -m "a \\" b \\" c" --no-verify', armed).deny).toBe(true);
});

test("double-quote escapes: \\\" \\\\ \\$ and \\` all stay inside the span, and --no-verify after it is still seen", () => {
  expect(classify('git commit -m "path\\\\dir" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "cost \\$5" --no-verify', armed).deny).toBe(true);
  expect(classify("git commit -m \"run \\`x\\`\" --no-verify", armed).deny).toBe(true);
});

test("a backslash before a NON-escapable character inside double quotes is literal, not an escape", () => {
  expect(classify('git commit -m "path\\ndir" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "path\\ndir"', armed).deny).toBe(false); // innocent direction
});

test("single quotes make every character literal, including a backslash that would otherwise escape", () => {
  expect(classify("git commit -m 'raw \\n text' --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m 'raw \\n text'", armed).deny).toBe(false); // innocent direction
});

test("an apostrophe inside a double-quoted message does not toggle quote state", () => {
  expect(classify("git commit -m \"it's fine\" --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m \"it's fine\"", armed).deny).toBe(false); // innocent direction
});

test("outside quotes, a backslash escapes the next character, keeping an escaped space in one word", () => {
  expect(classify("git commit -m fix\\ this --no-verify", armed).deny).toBe(true);
});

test("an unterminated quote does not throw, and a flag visible before it is still caught", () => {
  // A genuinely unterminated quote is malformed input a real shell also refuses to execute (syntax
  // error), so this guard's decision on content trapped inside it cannot itself enable a bypass;
  // what matters is that the tokenizer does not crash, and does not lose a flag that was never
  // inside the broken span to begin with.
  expect(() => classify('git commit --no-verify -m "unterminated', armed)).not.toThrow();
  expect(classify('git commit --no-verify -m "unterminated', armed).deny).toBe(true);
});

// A bare `VAR=value` prefix applies only to the command it directly prefixes; it must not leak
// across a `;`/`&&` chain to deny an unrelated later command.
test("a GIT_CONFIG_* prefix is scoped to its own segment, not the whole command", () => {
  expect(
    classify('GIT_CONFIG_COUNT=1 echo hi; git commit -m "unrelated"', armed).deny,
  ).toBe(false);
  expect(classify("GIT_CONFIG_COUNT=1 git commit -m x", armed).deny).toBe(true); // same segment: still denied
});

test("bestEffortCommand recovers only the command field, not an unrelated field in the same payload", () => {
  const payload =
    '{"tool_input":{"command":"pnpm test"},"description":"needs git commit hooks enabled"';
  expect(bestEffortCommand(payload)).toBe("pnpm test");
  expect(looksLikeCommitOrPush(bestEffortCommand(payload))).toBe(false);
});

test("bestEffortCommand returns null when no command field is present at all", () => {
  expect(bestEffortCommand("not json, no command key here")).toBe(null);
});

test("a parse failure with an unrelated field naming git commit does not deny the real command", () => {
  const result = spawnSync(execPath, [HOOK_PATH], {
    // Truncated JSON (no closing braces) so JSON.parse throws, but the command field itself is
    // harmless; only the description names a commit.
    input:
      '{"tool_input":{"command":"pnpm test"},"description":"needs git commit hooks enabled"',
    encoding: "utf8",
  });
  expect(result.stdout.trim()).toBe("");
});

// A bare `VAR=value` prefix applies only to the command it directly prefixes (segment-scoped,
// confirmed above). `export`, in contrast, marks the variable in the shell's OWN environment
// table, which every later command the shell spawns in the same invocation inherits — so it must
// propagate across segments, and an export of an unrelated variable must not.
test("export propagates a GIT_CONFIG_* override to a later command in the same chain", () => {
  expect(classify("export GIT_CONFIG_COUNT=1 && git commit -m x", armed).deny).toBe(true);
  expect(
    classify("export GIT_CONFIG_KEY_0=core.hooksPath; git push origin main", armed).deny,
  ).toBe(true);
  // The bare `export NAME` form (exporting a variable a prior segment already assigned) must
  // propagate identically to `export NAME=value`.
  expect(
    classify("GIT_CONFIG_COUNT=1; export GIT_CONFIG_COUNT; git commit -m x", armed).deny,
  ).toBe(true);
});

test("export of an unrelated variable does not deny, and a bare non-exported prefix still does not propagate", () => {
  expect(classify("export FOO=1 && git commit -m x", armed).deny).toBe(false);
  expect(classify('GIT_CONFIG_COUNT=1 echo hi; git commit -m "x"', armed).deny).toBe(false);
});
