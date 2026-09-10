// Denies the direct, typed forms of every known route around the git gates: --no-verify/-n on
// commit (never on push, where -n means --dry-run and mutates nothing), an inline
// `-c core.hooksPath=...` override or a `GIT_CONFIG_*` environment override of the same key, a
// `git config` write to core.hooksPath or extensions.worktreeConfig, and any commit or push at
// all in a repository with no gate installed. There is no bypass: an agent that cannot pass the
// gate stops and reports to the owner.
//
// This inspects the literal command STRING the harness is about to run; it does not run a shell.
// It cannot see a git invocation reached through a shell wrapper (`sh -c "..."`), command
// substitution, `eval`, an alias, or a shell function, because classifying an arbitrary shell
// string reliably would require executing one. Its purpose is to make the casual and accidental
// bypass impossible and the rule visible at the point of temptation, not to stop a determined
// caller working around the string match itself. Two other layers carry the actual guarantee: the
// git hooks (core.hooksPath) enforce on everything that reaches git through the normal path
// regardless of what this guard decides, and the remote enforces branch protection with required
// status checks, which is server-side and unaffected by anything local.
//
// The unarmed-repository denial is what makes the git-hook layer non-optional rather than
// best-effort: without it, a fresh clone with no hooks installed permits every commit and push
// silently.
//
// Quoted content (a commit message, most plainly) is one opaque token throughout: it is never
// read as a flag, a config key, or an environment override, and a `;`/`&`/`|` inside it is never
// a chain separator — a message that happens to quote `core.hooksPath` or a bypass flag as prose
// must stay an ordinary commit, and must not manufacture a fake segment either. Quoting follows
// the POSIX shell grammar exactly, not a quote-matching approximation of it: inside single quotes
// every character is literal with no escapes; inside double quotes a backslash escapes only
// `"`, `\`, `$`, and a backtick and is otherwise literal; outside quotes a backslash escapes
// whatever character follows it. An escaped quote inside a quoted span therefore never closes the
// span early — the one shape that DOES let content past it read as flags again is genuinely
// unterminated input (a quote with no matching close at all), which a real shell also refuses to
// run, so it is not a bypass this guard's decision on it can actually affect.
//
// Runs as a PreToolUse hook on every Bash call, so the common path stays cheap: nothing is parsed
// as a git invocation unless the command word's basename is `git`/`git.exe`/`git.cmd` (so a
// path-qualified or Windows-suffixed git is still recognised), and the one git subprocess this
// script itself spawns — to read core.hooksPath — is called lazily, only once classification
// actually reaches a commit or push with no bypass flag already found.
//
// An internal error fails OPEN for every command except one that plausibly names a git commit or
// push — --no-verify and the unarmed-repository check are the only two cases this guard uniquely
// covers (the git hooks do not exist to catch them), so silently allowing on this script's own
// malfunction would leave nothing enforcing the gate for exactly those two cases. Every other
// command fails open: a bug in this script must never brick a session.

import process from "node:process";
import { runGit } from "../../scripts/lib/run-git.mjs";

const ENV_ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;
const GIT_CONFIG_ENV_RE = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+)=/;

// Inside double quotes, a backslash escapes only these four characters (POSIX shell grammar); a
// backslash before anything else inside double quotes is a literal backslash, and the following
// character is processed normally rather than consumed as part of an escape.
const DOUBLE_QUOTE_ESCAPABLE = new Set(['"', "\\", "$", "`"]);

/**
 * Splits `command` into shell segments at an unquoted `;`, `&`, `&&`, `|`, `||`, or a newline, and
 * each segment into argv-style word tokens, implementing the POSIX shell quoting grammar rather
 * than approximating it: single quotes make every character literal with no escapes; double
 * quotes recognise only the four escapes above and are otherwise literal; outside quotes a
 * backslash escapes the next character verbatim, including whitespace, a quote character, or a
 * chain separator. A quoted span is therefore opaque end to end — no character inside one, escaped
 * or not, is ever read as a word boundary or a chain separator.
 *
 * @returns {string[][]} one array of tokens per segment.
 */
function segmentAndTokenize(command) {
  const segments = [];
  let tokens = [];
  let cur = "";
  let has = false;
  let quote = null; // null | '"' | "'"

  const flushToken = () => {
    if (has) {
      tokens.push(cur);
      cur = "";
      has = false;
    }
  };
  const flushSegment = () => {
    flushToken();
    if (tokens.length) segments.push(tokens);
    tokens = [];
  };

  const text = String(command);
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];

    if (quote === "'") {
      if (ch === "'") quote = null;
      else {
        cur += ch;
        has = true;
      }
      continue;
    }

    if (quote === '"') {
      if (ch === "\\" && i + 1 < text.length && DOUBLE_QUOTE_ESCAPABLE.has(text[i + 1])) {
        cur += text[i + 1];
        has = true;
        i++;
        continue;
      }
      if (ch === '"') {
        quote = null;
        continue;
      }
      cur += ch;
      has = true;
      continue;
    }

    // Outside any quote.
    if (ch === "\\") {
      if (i + 1 < text.length) {
        cur += text[i + 1];
        has = true;
        i++;
      } else {
        cur += "\\"; // a trailing backslash with nothing to escape stays literal
        has = true;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      has = true;
      continue;
    }
    if (ch === "\n" || ch === ";" || ch === "&" || ch === "|") {
      flushSegment();
      while (i + 1 < text.length && ";&|".includes(text[i + 1])) i++;
      continue;
    }
    if (/\s/.test(ch)) {
      flushToken();
      continue;
    }
    cur += ch;
    has = true;
  }
  flushSegment();
  return segments;
}

/** True when `token`'s basename (either slash style, minus a trailing .exe or .cmd suffix) is `git`. */
function isGitWord(token) {
  const base = token
    .split(/[\\/]/)
    .pop()
    .replace(/\.(exe|cmd)$/i, "");
  return base === "git";
}

/** Index of the git command word in `tokens`, skipping leading `NAME=value` env assignments; -1 if absent. */
function gitTokenIndex(tokens) {
  let i = 0;
  while (i < tokens.length && ENV_ASSIGNMENT.test(tokens[i])) i++;
  return i < tokens.length && isGitWord(tokens[i]) ? i : -1;
}

// Global flags that consume a SEPARATE next token as their value; an attached `--flag=value` form
// needs no special case, since it is already one token that `startsWith("-")` skips whole.
const GLOBAL_VALUE_FLAGS = new Set(["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path"]);

/** The git subcommand, skipping global flags (and their values) that precede it. */
function subcommand(gitArgs) {
  for (let i = 1; i < gitArgs.length; i++) {
    const t = gitArgs[i];
    if (GLOBAL_VALUE_FLAGS.has(t)) {
      i++;
      continue;
    }
    if (t.startsWith("-")) continue;
    return t;
  }
  return "";
}

/**
 * `args` with the value token belonging to `-m`/`--message` (or a combined short cluster carrying
 * `m`, e.g. `-am`) dropped — every other check in this file reads this instead of the raw args, so
 * a commit message can never be mistaken for a flag, a config key, or an inline override, no
 * matter what comes after it.
 */
function argsExcludingMessageValue(args) {
  const out = [];
  for (let i = 0; i < args.length; i++) {
    const t = args[i];
    out.push(t);
    if (t === "-m" || t === "--message") {
      i++;
      continue;
    }
    if (/^-[a-zA-Z]+$/.test(t) && t.includes("m")) {
      i++;
      continue;
    }
  }
  return out;
}

/**
 * `-n` is `--dry-run` for `git push` (mutates nothing) but skips the gate for `git commit`, so it
 * is only a bypass flag on `commit`. `--no-verify` skips the gate on both and is always a bypass.
 */
function hasBypassFlag(scanArgs, sub) {
  return scanArgs.some((t) => {
    if (t === "--no-verify") return true;
    if (sub !== "commit") return false;
    return /^-[a-zA-Z]+$/.test(t) && t.includes("n");
  });
}

// `git config --get`/`--get-all`/`--get-regexp`/`--get-urlmatch`/`-l`/`--list`/`--list-all` only
// read. `--unset`/`--unset-all`/`--add`/`--replace-all`/`--edit`/`-e`/`--rename-section`/
// `--remove-section` always write regardless of how many positional arguments follow. Absent
// either, a bare `git config <key>` (one positional argument) reads and `git config <key> <value>`
// (two) writes.
const CONFIG_READ_FLAGS = new Set([
  "--get",
  "--get-all",
  "--get-regexp",
  "--get-urlmatch",
  "-l",
  "--list",
  "--list-all",
]);
const CONFIG_WRITE_FLAGS = new Set([
  "--unset",
  "--unset-all",
  "--add",
  "--replace-all",
  "--edit",
  "-e",
  "--rename-section",
  "--remove-section",
]);
const CONFIG_VALUE_FLAGS = new Set(["--file", "-f", "--type", "-t", "--default"]);

/** Positional (non-flag) arguments to `git config`, in `scanArgs`, after the `config` token. */
function configPositionalArgs(scanArgs) {
  const out = [];
  let sawConfig = false;
  for (let i = 1; i < scanArgs.length; i++) {
    const t = scanArgs[i];
    if (!sawConfig) {
      if (t === "config") sawConfig = true;
      continue;
    }
    if (CONFIG_VALUE_FLAGS.has(t)) {
      i++;
      continue;
    }
    if (t.startsWith("-")) continue;
    out.push(t);
  }
  return out;
}

/**
 * True when a leading `NAME=value` assignment in THIS segment, immediately before the git word at
 * `gi`, sets a `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` environment variable
 * — git 2.31+ honours these for a single invocation, so they redirect core.hooksPath exactly like
 * `-c core.hooksPath=...` without that string ever appearing as a `-c` token. Scoped to the same
 * segment as `gitTokenIndex` already scopes the assignments themselves: a bare `VAR=value` prefix
 * applies only to the command it directly prefixes, never to a later command in a `;`/`&&` chain,
 * so checking other segments would deny an unrelated command that merely follows one.
 */
function hasGitConfigEnvOverride(tokens, gi) {
  for (let i = 0; i < gi; i++) {
    if (GIT_CONFIG_ENV_RE.test(tokens[i])) return true;
  }
  return false;
}

const GATE_CONFIG_REFUSAL =
  "Refused: core.hooksPath and extensions.worktreeConfig carry the gate. They are not overridable or reconfigurable by an agent. If the gate itself is wrong, stop and report it to the owner.";
const GIT_CONFIG_ENV_REFUSAL =
  "Refused: a GIT_CONFIG_* environment variable can redirect core.hooksPath for a single git invocation without the string ever appearing as a -c token. Setting any GIT_CONFIG_* variable around a commit or push is refused. If the gate itself is wrong, stop and report it to the owner.";
const NO_VERIFY_REFUSAL =
  "Refused: --no-verify (or -n on commit) skips the local gate. There is no bypass in this project. If the gate cannot pass, stop and report it to the owner.";
const UNARMED_REFUSAL =
  "Refused: this repository has no gate installed (core.hooksPath is unset). Run `pnpm install` to arm it before committing or pushing.";

const deny = (reason) => ({ deny: true, reason });

/** Whether this command must be refused, and what to tell the agent. */
export function classify(command, { hooksPathSet }) {
  const segments = segmentAndTokenize(command);

  // `hooksPathSet` may be a plain boolean (every existing test) or a lazy accessor (the real
  // entry point below, which must not spawn a git subprocess for a command that never reaches
  // this check). Evaluated at most once per `classify` call, and only on the path that needs it.
  let armedCache;
  const isArmed = () => {
    if (armedCache === undefined) {
      armedCache = typeof hooksPathSet === "function" ? hooksPathSet() : hooksPathSet;
    }
    return armedCache;
  };

  for (const tokens of segments) {
    const gi = gitTokenIndex(tokens);
    if (gi === -1) continue;
    const gitArgs = tokens.slice(gi);
    const scanArgs = argsExcludingMessageValue(gitArgs);
    const sub = subcommand(gitArgs);

    // An inline `-c key=value` (or a `key=value` positional to `config`) is always a write for the
    // scope it targets, regardless of subcommand.
    if (
      scanArgs.some(
        (t) => t.startsWith("core.hooksPath=") || t.startsWith("extensions.worktreeConfig="),
      )
    ) {
      return deny(GATE_CONFIG_REFUSAL);
    }

    if (sub === "config") {
      const targetsGateConfig = scanArgs.some(
        (t) => t === "core.hooksPath" || t === "extensions.worktreeConfig",
      );
      if (targetsGateConfig) {
        const hasWriteFlag = scanArgs.some((t) => CONFIG_WRITE_FLAGS.has(t));
        const hasReadFlag = scanArgs.some((t) => CONFIG_READ_FLAGS.has(t));
        const positional = configPositionalArgs(scanArgs);
        const isRead = !hasWriteFlag && (hasReadFlag || positional.length <= 1);
        if (!isRead) return deny(GATE_CONFIG_REFUSAL);
      }
      continue; // `config` is never itself a commit or push
    }

    if (sub !== "commit" && sub !== "push") continue;

    if (hasGitConfigEnvOverride(tokens, gi)) return deny(GIT_CONFIG_ENV_REFUSAL);
    if (hasBypassFlag(scanArgs, sub)) return deny(NO_VERIFY_REFUSAL);
    if (!isArmed()) return deny(UNARMED_REFUSAL);
  }
  return { deny: false, reason: "" };
}

/** True when either the worktree or repository scope has core.hooksPath set to a non-empty value. */
function queryHooksPathSet() {
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

/** Coarse, deliberately permissive match used only on the fail-closed path: true when raw,
 * possibly-unparseable text plausibly names a git commit or push, so an internal error while
 * evaluating it denies rather than silently allows. */
export function looksLikeCommitOrPush(text) {
  return /\bgit\b[\s\S]*\b(commit|push)\b/.test(String(text));
}

/**
 * Recovers just the `command` field's value from raw, possibly-truncated or malformed JSON text,
 * without requiring the payload to parse as a whole — used only on the fail-closed path, where the
 * coarse commit/push match must be scoped to the command, never to an unrelated field (a
 * `description` mentioning "git commit hooks" must not deny a `pnpm test` invocation). Returns
 * `null` when no `"command"` key is found at all, the only case the caller falls back to scanning
 * the whole payload.
 */
export function bestEffortCommand(raw) {
  const match = /"command"\s*:\s*"((?:[^"\\]|\\.)*)"?/.exec(String(raw));
  if (!match) return null;
  try {
    // The regex already captured a well-formed JSON string body (or the un-terminated remainder of
    // one); re-parsing it through JSON's own escape rules is simpler and more correct than
    // reimplementing them, and a truncated payload's missing closing quote is supplied here.
    return JSON.parse('"' + match[1] + '"');
  } catch {
    return match[1]; // an escape sequence too broken to parse; the raw captured text is still
    // narrower than the whole payload, which is what this function exists to guarantee.
  }
}

if (isMainEntry()) {
  let raw = "";
  process.stdin.on("data", (chunk) => {
    raw += chunk;
  });
  process.stdin.on("end", () => {
    let command;
    try {
      command = JSON.parse(raw)?.tool_input?.command ?? "";
      const verdict = classify(command, { hooksPathSet: queryHooksPathSet });
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
      // `command` may already hold the real value if JSON.parse succeeded but classify() itself
      // threw; otherwise recover it best-effort from the raw text, and only scan the whole payload
      // when no command field can be found in it at all.
      const recovered = command ?? bestEffortCommand(raw);
      const scanTarget = recovered ?? raw;
      if (looksLikeCommitOrPush(scanTarget)) {
        process.stdout.write(
          JSON.stringify({
            hookSpecificOutput: {
              hookEventName: "PreToolUse",
              permissionDecision: "deny",
              permissionDecisionReason:
                "Refused: the guard hit an internal error evaluating a command that names a git commit or push, and cannot safely allow it. Stop and report this to the owner rather than retrying with a bypass flag.",
            },
          }),
        );
      }
    }
    process.exit(0);
  });
}
