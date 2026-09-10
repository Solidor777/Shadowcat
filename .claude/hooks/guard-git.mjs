// Denies every route around the git gates, and denies git writes in a repository with no gates
// installed at all.
//
// There is no bypass: an agent that cannot pass the gate stops and reports to the owner. The
// unarmed-repository denial is what makes the git-hook layer non-optional rather than
// best-effort — without it, a fresh clone with no hooks installed permits every commit and push
// silently, and "impossible to bypass" degrades to "usually enforced".
//
// Flag scanning stops at -m/--message: a commit message legitimately contains substrings like
// "-n", and treating message text as flags would deny an honest commit.
//
// Runs as a PreToolUse hook on every Bash call, so it must stay cheap on the common path: no git
// invocation at all unless the command actually looks like `git commit` or `git push`, and a
// single git query even then. Any internal error here is swallowed and treated as "allow" rather
// than "deny" — the git hooks (core.hooksPath) are the enforcement layer this backs up, and stay
// in force independently of this script; a guard that denies on its own malfunction would block
// every Bash call in the session, including the ones needed to diagnose it.

import process from "node:process";
import { runGit } from "../../scripts/lib/run-git.mjs";

const SEGMENT = /[;&|]{1,2}|\n/;

function isGit(tokens) {
  return tokens[0] === "git";
}

/** The git subcommand, skipping `-c key=value` pairs and other leading flags. */
function subcommand(tokens) {
  for (let i = 1; i < tokens.length; i++) {
    const t = tokens[i];
    if (t === "-c") {
      i++;
      continue;
    }
    if (t.startsWith("-")) continue;
    return t;
  }
  return "";
}

/** Flags preceding the first `-m`/`--message` token, or a short flag cluster ending in `m`. */
function flagsBeforeMessage(tokens) {
  const out = [];
  for (let i = 1; i < tokens.length; i++) {
    const t = tokens[i];
    if (t === "-m" || t === "--message") break;
    if (/^-[a-zA-Z]+$/.test(t) && t.includes("m")) break;
    if (t.startsWith("-")) out.push(t);
  }
  return out;
}

// `git config --get`/`--get-all`/`--get-regexp`/`--get-urlmatch`/`-l`/`--list`/`--list-all` only
// read; every other form of `git config <key> ...` either sets or removes the key. Reads of
// core.hooksPath and extensions.worktreeConfig are ordinary diagnostic commands and must stay
// allowed — only a write to either is a bypass route.
const CONFIG_READ_FLAGS = new Set([
  "--get",
  "--get-all",
  "--get-regexp",
  "--get-urlmatch",
  "-l",
  "--list",
  "--list-all",
]);

/** Whether this command must be refused, and what to tell the agent. */
export function classify(command, { hooksPathSet }) {
  for (const segment of String(command).split(SEGMENT)) {
    const tokens = segment.trim().split(/\s+/).filter(Boolean);
    if (!isGit(tokens)) continue;

    // An inline `-c key=value` (or a bare `key=value` argument to `config`) is always a write for
    // the scope it targets, never a read, so it is denied unconditionally.
    if (
      tokens.some(
        (t) => t.startsWith("core.hooksPath=") || t.startsWith("extensions.worktreeConfig="),
      )
    ) {
      return {
        deny: true,
        reason:
          "Refused: core.hooksPath and extensions.worktreeConfig carry the gate. They are not overridable or reconfigurable by an agent. If the gate itself is wrong, stop and report it to the owner.",
      };
    }
    const targetsGateConfig = tokens.some(
      (t) => t === "core.hooksPath" || t === "extensions.worktreeConfig",
    );
    if (targetsGateConfig && !tokens.some((t) => CONFIG_READ_FLAGS.has(t))) {
      return {
        deny: true,
        reason:
          "Refused: core.hooksPath and extensions.worktreeConfig carry the gate. They are not overridable or reconfigurable by an agent. If the gate itself is wrong, stop and report it to the owner.",
      };
    }

    const sub = subcommand(tokens);
    if (sub !== "commit" && sub !== "push") continue;

    const flags = flagsBeforeMessage(tokens);
    const bypass = flags.some(
      (f) => f === "--no-verify" || (/^-[a-zA-Z]+$/.test(f) && f.includes("n")),
    );
    if (bypass) {
      return {
        deny: true,
        reason:
          "Refused: --no-verify (or -n) skips the local gate. There is no bypass in this project. If the gate cannot pass, stop and report it to the owner.",
      };
    }
    if (!hooksPathSet) {
      return {
        deny: true,
        reason:
          "Refused: this repository has no gate installed (core.hooksPath is unset). Run `pnpm install` to arm it before committing or pushing.",
      };
    }
  }
  return { deny: false, reason: "" };
}

/** True when either the worktree or repository scope has core.hooksPath set to a non-empty value. */
function hooksPathSet() {
  for (const args of [
    ["config", "--get", "--worktree", "core.hooksPath"],
    ["config", "--get", "core.hooksPath"],
  ]) {
    const result = runGit(args, "whether core.hooksPath is set");
    if (result.ok && result.stdout) return true;
  }
  return false;
}

/** True only when this module is being run directly as the hook entry point, not imported for its exports. */
function isMainEntry() {
  const invoked = process.argv[1];
  if (!invoked) return false;
  return import.meta.url.endsWith(invoked.replace(/\\/g, "/").split("/").pop());
}

if (isMainEntry()) {
  let raw = "";
  process.stdin.on("data", (chunk) => {
    raw += chunk;
  });
  process.stdin.on("end", () => {
    try {
      const command = JSON.parse(raw)?.tool_input?.command ?? "";
      const verdict = classify(command, { hooksPathSet: hooksPathSet() });
      if (verdict.deny) {
        process.stdout.write(
          JSON.stringify({
            hookSpecificOutput: {
              hookEventName: "PreToolUse",
              permissionDecision: "deny",
              permissionDecisionReason: verdict.reason,
            },
          }),
        );
      }
    } catch {
      // Fail open: an internal error here must never block a Bash call. The git hooks enforce
      // the gate independently of this script.
    }
    process.exit(0);
  });
}
