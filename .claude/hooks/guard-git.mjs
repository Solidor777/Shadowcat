// Denies the direct, typed forms of every known route around the git gates: --no-verify/-n on
// commit (never on push, where -n means --dry-run and mutates nothing), an inline
// `-c core.hooksPath=...` override or a `GIT_CONFIG_*` environment override of the same key, a
// `git config` write to core.hooksPath or extensions.worktreeConfig, and any commit or push at
// all in a repository with no gate installed. There is no bypass: an agent that cannot pass the
// gate stops and reports to the owner.
//
// This inspects the literal command STRING the harness is about to run; it does not run a shell.
// It cannot see a git invocation reached through a shell wrapper (`sh -c "..."`, `env NAME=value
// git ...`, `command git`, `exec git`), command substitution, `eval`, an alias, or a shell
// function, because classifying an arbitrary shell string reliably would require executing one.
// For the same reason it recognises a command word only at the START of a segment (after any
// leading `NAME=value` assignments): a git word behind a subshell `(`, a brace group `{`, or a
// reserved word (`then`, `do`, `!`, `time`) is a wrapper of the same family and is not seen. Its
// purpose is to make the casual and accidental bypass impossible and the rule visible at the
// point of temptation, not to stop a determined caller working around the string match itself.
// Two other layers carry the actual guarantee: the git hooks (core.hooksPath) enforce on
// everything that reaches git through the normal path regardless of what this guard decides, and
// the remote enforces branch protection with required status checks, which is server-side and
// unaffected by anything local.
//
// The unarmed-repository denial is what makes the git-hook layer non-optional rather than
// best-effort: without it, a fresh clone with no hooks installed permits every commit and push
// silently.
//
// Quoted content (a commit message, most plainly) is one opaque token throughout: it is never
// read as a flag, a config key, or an environment override, and a `;`/`&`/`|` inside it is never
// a chain separator — a message that happens to quote `core.hooksPath` or a bypass flag as prose
// must stay an ordinary commit, and must not manufacture a fake segment either. The tokenizer
// implements the POSIX Shell Command Language's quoting and token-recognition rules as a
// grammar, enumerated at `segmentAndTokenize`; a false denial here blocks real work with no
// switch to turn it off, so every rule that decides where a quoted span ENDS is implemented,
// including the ones that only matter for the innocent direction. What it does NOT do is
// EXPAND: `$var`, `$(…)`, `` `…` ``, `${…}` and `$((…))` are tracked only for their extent, so the
// enclosing quote state stays correct across them, and their text is kept verbatim as opaque
// content — a flag produced by an expansion is the command-substitution boundary above. Named
// bash extensions outside the POSIX grammar, deliberately not implemented: `$'…'` (ANSI-C
// quoting — the `$` is read as a literal and the quote after it as an ordinary single quote, so
// a word built with it is misread), `$"…"` (locale translation), brace expansion (`{a,b}`),
// and a `case` pattern's unbalanced `)` inside `$(…)`, whose extent is found by parenthesis
// balance. The one shape that lets content past a quote read as flags again is genuinely
// unterminated input (a quote with no matching close at all), which a real shell also refuses
// to run, so it is not a bypass this guard's decision on it can actually affect.
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
// Matches a bare `export` argument naming a GIT_CONFIG_* variable, with or without a `=value` —
// `export GIT_CONFIG_KEY_0` (exporting an already-assigned shell variable) redirects the gate
// exactly like `export GIT_CONFIG_KEY_0=...` (assigning and exporting in one step); only the
// value's presence differs, never whether the override applies.
const GIT_CONFIG_EXPORT_ARG_RE = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+)(=.*)?$/;

// Inside double quotes a backslash escapes exactly these four characters plus <newline> (which
// is a line continuation, handled separately because the pair is REMOVED rather than replaced);
// before anything else it is a literal backslash and the following character is processed
// normally rather than consumed as part of an escape.
const DOUBLE_QUOTE_ESCAPABLE = new Set(['"', "\\", "$", "`"]);

// Characters that end an unquoted word: whitespace, and the operator characters. `(`/`)` are
// listed because they delimit a here-document delimiter word; at the top level they are kept as
// ordinary word characters (a subshell is the command-position boundary in the header).
const HEREDOC_DELIMITER_END = /[\s;&|<>()]/;

/**
 * Splits `command` into shell segments and each segment into argv-style word tokens, following
 * the POSIX Shell Command Language (IEEE Std 1003.1-2017, XCU chapter 2: Quoting, Token
 * Recognition, Command Substitution, Here-Document). The rules, each of which the
 * test suite exercises in both the denied and the allowed direction:
 *
 *   Escape Character — Outside quotes a backslash preserves the literal value of the next character, which
 *           may be a quote, whitespace, or an operator. Backslash-<newline> is a line
 *           continuation: both characters are removed and nothing is inserted, so it neither
 *           ends a word nor separates two. A trailing backslash at end of input is literal.
 *   Single-Quotes — Inside single quotes every character is literal, including backslash and newline;
 *           a single quote cannot occur inside single quotes.
 *   Double-Quotes — Inside double quotes every character is literal except `$`, backquote and
 *           backslash. Backslash escapes only `$`, backquote, `"`, backslash and <newline> (the
 *           last as a line continuation, removed); before any other character it is literal.
 *           A `${…}` inside double quotes carries its own balanced quoting, and a `$(…)` or
 *           backquoted substitution is parsed by the shell grammar in its own right (Command Substitution), so
 *           a double quote inside either never closes the enclosing span.
 *   Token Recognition — Adjacent quoted and unquoted parts form one word; empty quotes form an empty word.
 *           An unquoted `#` at the start of a word begins a comment that runs to the newline and
 *           is discarded whole — nothing inside it, a quote character or a backslash-<newline>
 *           included, has any effect. Unquoted `;`, `&`, `&&`, `|`, `||` and <newline> end a
 *           segment; `>&`, `<&`, `>|` (and bash's `&>`, `&>>`) are redirection operators, not
 *           separators, so the arguments after them still belong to the same command.
 *   Here-Document — `<<` or `<<-` followed by a delimiter word (quoted or not) opens a here-document
 *           whose body starts after the next unquoted <newline> and ends at the first line equal
 *           to the delimiter (leading tabs stripped under `<<-`); with an unquoted delimiter a
 *           body line ending in backslash joins the next line before the comparison. The body is
 *           data, never a segment. `<<<` is a here-string and opens nothing.
 *
 * A substitution's extent is tracked so quote state survives it; its text is kept verbatim in
 * the word and is never expanded (see the header for the named exclusions).
 *
 * @returns {string[][]} one array of tokens per segment.
 */
export function segmentAndTokenize(command) {
  const text = String(command);
  const n = text.length;
  const segments = [];
  let tokens = [];
  let cur = "";
  let has = false;

  // Enclosing contexts, innermost last. `single`/`double`/`backtick` are quote spans; `paren`
  // (`$(`, balanced to its `)`) and `brace` (`${`, to its `}`) are substitutions whose interior
  // is read under the unquoted rules again. `substDepth` counts the substitution frames
  // (`paren`/`brace`/`backtick`) so the word-building code knows to keep text verbatim.
  const stack = [];
  let substDepth = 0;
  const top = () => stack[stack.length - 1];
  const push = (frame) => {
    stack.push(frame);
    if (frame.kind !== "single" && frame.kind !== "double") substDepth++;
  };
  const pop = () => {
    const frame = stack.pop();
    if (frame.kind !== "single" && frame.kind !== "double") substDepth--;
  };
  // The unquoted rules apply at the top level and directly inside a `$(`/`${` frame.
  const mode = () => {
    const t = top();
    if (!t || t.kind === "paren" || t.kind === "brace") return "unquoted";
    return t.kind;
  };

  const add = (s) => {
    cur += s;
    has = true;
  };
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

  // True at a position where an unquoted `#` starts a comment: the start of a word.
  let atWordStart = true;
  // True when the last character added to the current top-level word is an unquoted `>` or `<`,
  // so a following `&` or `|` completes a redirection operator instead of separating segments.
  let prevRedir = false;
  // Here-documents opened on the current line, consumed in order after its unquoted newline.
  let pendingHeredocs = [];

  /**
   * Reads the delimiter word after a `<<`/`<<-` at `i`; returns the index past it. Any quoting
   * in the word marks the body literal (no line joining), and the quotes themselves are not
   * part of the delimiter.
   */
  const readHeredocOperator = (i) => {
    let j = i + 2;
    let stripTabs = false;
    if (text[j] === "-") {
      stripTabs = true;
      j++;
    }
    while (j < n && (text[j] === " " || text[j] === "\t")) j++;
    let delimiter = "";
    let quoted = false;
    while (j < n) {
      const c = text[j];
      if (c === "'") {
        quoted = true;
        const close = text.indexOf("'", j + 1);
        const end = close === -1 ? n : close;
        delimiter += text.slice(j + 1, end);
        j = end + 1;
        continue;
      }
      if (c === '"') {
        quoted = true;
        j++;
        while (j < n && text[j] !== '"') {
          if (text[j] === "\\" && j + 1 < n) {
            if (text[j + 1] === "\n") {
              j += 2;
              continue;
            }
            if (DOUBLE_QUOTE_ESCAPABLE.has(text[j + 1])) {
              delimiter += text[j + 1];
              j += 2;
              continue;
            }
          }
          delimiter += text[j];
          j++;
        }
        j++;
        continue;
      }
      if (c === "\\") {
        if (j + 1 < n && text[j + 1] === "\n") {
          j += 2;
          continue;
        }
        quoted = true;
        if (j + 1 < n) delimiter += text[j + 1];
        j += 2;
        continue;
      }
      if (HEREDOC_DELIMITER_END.test(c)) break;
      delimiter += c;
      j++;
    }
    if (delimiter) pendingHeredocs.push({ delimiter, quoted, stripTabs });
    return j;
  };

  /** Consumes every pending here-document body starting at line index `start`; returns the index after the last one. */
  const consumeHeredocBodies = (start) => {
    let pos = start;
    for (const { delimiter, quoted, stripTabs } of pendingHeredocs) {
      let logical = "";
      while (pos < n) {
        const nl = text.indexOf("\n", pos);
        const lineEnd = nl === -1 ? n : nl;
        const line = text.slice(pos, lineEnd);
        pos = nl === -1 ? n : nl + 1;
        const trailing = /\\*$/.exec(line)[0].length;
        if (!quoted && trailing % 2 === 1) {
          logical += line.slice(0, -1);
          continue; // joined with the next line before the comparison
        }
        logical += line;
        if ((stripTabs ? logical.replace(/^\t+/, "") : logical) === delimiter) break;
        logical = "";
      }
    }
    pendingHeredocs = [];
    if (substDepth > 0) add(text.slice(start, pos));
    return pos;
  };

  let i = 0;
  while (i < n) {
    const ch = text[i];
    const m = mode();
    const verbatim = substDepth > 0;

    if (m === "single") {
      if (ch === "'") {
        pop();
        if (verbatim) add("'");
      } else add(ch);
      i++;
      continue;
    }

    if (m === "double") {
      if (ch === "\\" && i + 1 < n) {
        const nx = text[i + 1];
        if (nx === "\n") {
          i += 2; // line continuation: removed outright
          continue;
        }
        if (DOUBLE_QUOTE_ESCAPABLE.has(nx)) {
          add(verbatim ? ch + nx : nx);
          i += 2;
          continue;
        }
        add(ch); // literal backslash; `nx` is processed on the next iteration
        i++;
        continue;
      }
      if (ch === '"') {
        pop();
        if (verbatim) add('"');
        i++;
        continue;
      }
      if (ch === "$" && (text[i + 1] === "(" || text[i + 1] === "{")) {
        push({ kind: text[i + 1] === "(" ? "paren" : "brace", depth: 1 });
        add(ch + text[i + 1]);
        atWordStart = text[i + 1] === "(";
        i += 2;
        continue;
      }
      if (ch === "`") {
        push({ kind: "backtick" });
        add(ch);
        i++;
        continue;
      }
      add(ch);
      i++;
      continue;
    }

    if (m === "backtick") {
      if (ch === "\\" && i + 1 < n) {
        add(ch + text[i + 1]);
        i += 2;
        continue;
      }
      if (ch === "`") pop();
      add(ch);
      i++;
      continue;
    }

    // Unquoted rules: the top level, or the interior of a `$(`/`${` frame.
    const frame = top();
    if (ch === "\\") {
      if (i + 1 < n) {
        const nx = text[i + 1];
        if (nx === "\n") {
          i += 2; // line continuation: removed, and not a word boundary
          continue;
        }
        add(verbatim ? ch + nx : nx);
        i += 2;
      } else {
        add(ch); // a trailing backslash with nothing to escape stays literal
        i++;
      }
      atWordStart = false;
      prevRedir = false;
      continue;
    }
    if (ch === "'" || ch === '"') {
      push({ kind: ch === "'" ? "single" : "double" });
      if (verbatim) add(ch);
      else has = true; // an empty quoted span is still a (possibly empty) word
      atWordStart = false;
      prevRedir = false;
      i++;
      continue;
    }
    if (ch === "`") {
      push({ kind: "backtick" });
      add(ch);
      atWordStart = false;
      prevRedir = false;
      i++;
      continue;
    }
    if (ch === "$") {
      if (text[i + 1] === "(" || text[i + 1] === "{") {
        push({ kind: text[i + 1] === "(" ? "paren" : "brace", depth: 1 });
        add(ch + text[i + 1]);
        atWordStart = text[i + 1] === "(";
        i += 2;
      } else {
        add(ch);
        atWordStart = false;
        i++;
      }
      prevRedir = false;
      continue;
    }
    if (ch === "#" && atWordStart && (!frame || frame.kind === "paren")) {
      const nl = text.indexOf("\n", i);
      const end = nl === -1 ? n : nl;
      if (verbatim) add(text.slice(i, end));
      i = end; // the newline itself is handled below on the next iteration
      continue;
    }
    if (ch === "\n") {
      if (verbatim) add(ch);
      else flushSegment();
      atWordStart = true;
      prevRedir = false;
      i = pendingHeredocs.length ? consumeHeredocBodies(i + 1) : i + 1;
      continue;
    }
    if (frame && frame.kind === "paren" && (ch === "(" || ch === ")")) {
      if (ch === "(") frame.depth++;
      else if (--frame.depth === 0) pop();
      add(ch);
      atWordStart = frame.depth > 0; // a closed substitution continues the enclosing word
      i++;
      continue;
    }
    if (frame && frame.kind === "brace" && ch === "}") {
      pop();
      add(ch);
      atWordStart = false;
      i++;
      continue;
    }
    if (ch === "<" && text[i + 1] === "<" && text[i + 2] === "<") {
      flushToken();
      add("<<<"); // here-string operator: its operand is an ordinary word, not a body
      flushToken();
      atWordStart = true;
      prevRedir = false;
      i += 3;
      continue;
    }
    if (ch === "<" && text[i + 1] === "<") {
      const j = readHeredocOperator(i);
      if (verbatim) add(text.slice(i, j));
      else flushToken();
      atWordStart = true;
      prevRedir = false;
      i = j;
      continue;
    }
    if (verbatim) {
      add(ch);
      atWordStart = HEREDOC_DELIMITER_END.test(ch);
      i++;
      continue;
    }
    if (ch === ";" || ch === "&" || ch === "|") {
      const redirection = ch !== ";" && (prevRedir || (ch === "&" && text[i + 1] === ">"));
      if (!redirection) {
        flushSegment();
        while (i + 1 < n && ";&|".includes(text[i + 1])) i++;
        atWordStart = true;
        prevRedir = false;
        i++;
        continue;
      }
    }
    if (/\s/.test(ch)) {
      flushToken();
      atWordStart = true;
      prevRedir = false;
      i++;
      continue;
    }
    add(ch);
    atWordStart = false;
    prevRedir = ch === ">" || ch === "<";
    i++;
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

/**
 * Index of the segment's command word — the first token after the leading run of `NAME=value`
 * assignments — or `tokens.length` when the segment is assignments only. Every check that asks
 * "is this segment's command X" reads this, so a word appearing anywhere else in the segment
 * (an argument to `printf`, say) is never mistaken for the command.
 */
function commandWordIndex(tokens) {
  let i = 0;
  while (i < tokens.length && ENV_ASSIGNMENT.test(tokens[i])) i++;
  return i;
}

/** Index of the git command word in `tokens`, skipping leading `NAME=value` env assignments; -1 if absent. */
function gitTokenIndex(tokens) {
  const i = commandWordIndex(tokens);
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
 * (with no `export`) applies only to the command it directly prefixes, never to a later command in
 * a `;`/`&&` chain, so checking other segments would deny an unrelated command that merely follows
 * one — unlike `export`, below, which genuinely does propagate.
 */
function hasGitConfigEnvOverride(tokens, gi) {
  for (let i = 0; i < gi; i++) {
    if (GIT_CONFIG_ENV_RE.test(tokens[i])) return true;
  }
  return false;
}

/**
 * True when this segment's COMMAND is `export` and it names a GIT_CONFIG_* variable — either
 * `export NAME=value` (assigns and exports together) or a bare `export NAME` (exports a variable
 * a prior segment already assigned). The command position is the same one `gitTokenIndex` uses,
 * so `export` appearing as another command's argument (`printf export ...`) is never read as
 * one. Unlike a bare `VAR=value` prefix, `export` marks the variable in the shell's own
 * environment table, which every command the shell spawns AFTER it inherits — so this is checked
 * across every segment preceding the git invocation, not just the one containing it. A `--`
 * ends the options and is skipped; any other option stops the scan, since `-n` removes the
 * export attribute, `-p` prints, and `-f` names functions — none of them exports a variable.
 */
function segmentExportsGitConfigVar(tokens) {
  const ci = commandWordIndex(tokens);
  if (ci >= tokens.length || tokens[ci] !== "export") return false;
  for (let j = ci + 1; j < tokens.length; j++) {
    const t = tokens[j];
    if (t === "--") continue;
    if (GIT_CONFIG_EXPORT_ARG_RE.test(t)) return true;
    if (!/^[A-Za-z_][A-Za-z0-9_]*(=.*)?$/.test(t)) break; // an option, or the end of the argument list
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

  for (let segIdx = 0; segIdx < segments.length; segIdx++) {
    const tokens = segments[segIdx];
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

    const envOverride =
      hasGitConfigEnvOverride(tokens, gi) ||
      segments.slice(0, segIdx).some((seg) => segmentExportsGitConfigVar(seg));
    if (envOverride) return deny(GIT_CONFIG_ENV_REFUSAL);
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
