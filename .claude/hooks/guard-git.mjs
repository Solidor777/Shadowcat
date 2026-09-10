// A best-effort pre-filter over the git invocations an agent types. It denies the direct, typed
// forms of every known route around the git gates:
//   - `--no-verify` on commit or push, in every spelling git accepts: any unambiguous
//     abbreviation (`--no-veri`), or `-n` anywhere in a short-option bundle on commit (`-an`,
//     `-nm x`). On push `-n` is `--dry-run`, which mutates nothing.
//   - an inline configuration of a gate-carrying key on ANY subcommand, through `-c`,
//     `--config-env`, or a `GIT_CONFIG_*` environment variable: `core.hooksPath`,
//     `extensions.worktreeConfig`, and the include directives `include.path` /
//     `includeIf.<condition>.path` (an included file carries the gate key at command-line
//     precedence, which outranks every file scope).
//   - a `git config` write to a gate key in any spelling, scope or file; a section-level write
//     (`--remove-section`, `--rename-section`) to the `core` or `extensions` section; and
//     `git config --edit`, whose scripted editor is a write with no visible key.
//   - an inline alias (`-c alias.<name>=…`) invoked in the same command, classified through its
//     expansion.
//   - any commit or push at all in a repository with no gate installed.
// There is no bypass: an agent that cannot pass the gate stops and reports to the owner.
//
// WHAT THIS LAYER IS. It inspects the literal command STRING the harness is about to run; it does
// not run a shell and it does not run git's own parser. Its surface is unbounded, so it is not
// and cannot be a complete barrier: it does not see a git invocation reached through command
// substitution, `eval`, a shell alias or function, a wrapper script, `env`, `command`, `exec`,
// a subshell `(…)`, a brace group `{ …; }`, a reserved word (`then`, `do`, `!`, `time`), process
// substitution `<(…)`, or an alias persisted in a config file, and it recognises a command word
// only at the START of a segment after any leading `NAME=value` assignments. Nothing here stops a
// determined caller working around the string match; the purpose is to make the casual and
// accidental bypass impossible and the rule visible at the point of temptation. The layers that
// do not depend on parsing carry the actual guarantee: the git hooks (`core.hooksPath`) run on
// everything that reaches git through the normal path whatever this guard decides, and the
// remote's branch protection with required status checks is server-side and unaffected by
// anything local. The unarmed-repository denial is what makes the hook layer non-optional
// rather than best-effort: without it, a fresh clone with no hooks installed permits every
// commit and push silently.
//
// REGISTRATION. Runs as a PreToolUse hook on EVERY harness tool. The set of tools that execute a
// command string (Bash, PowerShell, Monitor, any added later) is not enumerable from this file,
// and a matcher that misses one loses the whole layer through that tool, so the hook is
// registered for all of them and is a no-op for any payload without a `tool_input.command`
// string. The common path stays cheap: nothing is parsed as a git invocation unless a segment's
// command word has the basename `git`/`git.exe`/`git.cmd`, and the one git subprocess this
// script spawns — to read core.hooksPath — runs lazily, only once classification reaches a
// commit or push with no other refusal already found.
//
// GRAMMAR BOUNDARY. The tokenizer implements the POSIX Shell Command Language (see
// `segmentAndTokenize`), which is the grammar of the Bash tool and of git's own `!`-alias and
// hook execution. The PowerShell tool runs PowerShell, whose quoting differs (backtick escapes,
// doubled quotes inside a quoted string, no backslash escapes, `--%`): a PowerShell command
// whose quoting the POSIX rules read differently can misclassify in either direction, and only a
// PowerShell tokenizer would close that. Plain spellings with no quoting disagreement classify
// identically in both shells.
//
// GIT GRAMMAR. Every rule about git's own argument syntax is taken from git's documentation and
// measured against git, never inferred from examples — the alternative reimplements git's parser
// from imagination one spelling at a time. The rules, each named at the code that
// implements it:
//   git(1) OPTIONS — the front end's own options (`-c`, `--config-env`, `-C`, `--git-dir`, …),
//     matched by exact string: no bundling, no abbreviation; parsing ends at the first word
//     that is not an option, which is the subcommand (`subcommandIndex`).
//   parse-options API (Documentation/technical/api-parse-options) — every subcommand's options:
//     short options bundle, a short option taking a value takes the rest of its bundle or, if
//     the bundle ends, the next word; an optional value takes only the rest of the bundle; a long
//     option takes `--name=value` or `--name value` (required) / `--name=value` only (optional);
//     long names may be abbreviated to any prefix that names exactly one option, exact spellings
//     win, and an ambiguous prefix is an error; `--no-name` negates and takes no value, and an
//     option named `no-x` is negated by `--x`; `--` ends options and everything after it is an
//     operand (`walkOptions`, `resolveLongOption`, the `*_OPTIONS` tables).
//   git-config(1) — section and variable names are case-insensitive, subsection names are not
//     (`foldConfigKey`); `-c name` with no `=` and `-c name=` are both writes of `name`
//     (`inlineConfigPairs`); the subcommand modes (`list`/`get` read; `set`/`unset`/
//     `rename-section`/`remove-section`/`edit` write), the deprecated flag modes, and the
//     positional rule with no mode (one operand reads, two or more write) (`configWritesGate`).
//   git(1) ENVIRONMENT — `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` and
//     `GIT_CONFIG_PARAMETERS` add configuration at command-line precedence. Their NAMES are
//     matched case-insensitively: environment variable names are case-insensitive on Windows,
//     so `git_config_count=1` redirects the gate there, and a lowercase spelling is inert
//     elsewhere, so the wider match denies nothing that would have run.
//   git-config(1) `alias.<name>` — an alias is expanded by splitting its value into words under
//     shell quoting rules and substituting them for the subcommand; a value beginning with `!`
//     runs as a shell command with the remaining arguments appended. An alias cannot hide
//     `commit`, `push` or `config` (git ignores an alias that shadows a builtin), so expansion
//     stops at those (the alias arm of `classifyGitSegment`).
//   Not modelled, by design: a subcommand's option not in its table is read as a flag taking no
//     value, which can only produce a false denial (a value read as a flag), never a bypass; the
//     tables are pinned against the installed git's own `-h` output by the test suite. A
//     `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` file is inert against the gate, which is armed at
//     worktree scope, the highest file scope.
//
// Quoted content (a commit message, most plainly) is one opaque token throughout: it is never
// read as a flag, a config key, or an environment override, and a `;`/`&`/`|` inside it is never
// a chain separator — a message that happens to quote `core.hooksPath` or a bypass flag as prose
// must stay an ordinary commit, and must not manufacture a fake segment either. A false denial
// blocks real work with no switch to turn it off, so every rule that decides where a quoted span
// ENDS is implemented, including the ones that only matter for the innocent direction. What the
// tokenizer does NOT do is EXPAND: `$var`, `$(…)`, `` `…` ``, `${…}` and `$((…))` are tracked
// only for their extent, so the enclosing quote state stays correct across them, and their text
// is kept verbatim as opaque content — a flag produced by an expansion is the command-substitution
// boundary above. Named bash extensions outside the POSIX grammar, deliberately not implemented:
// `$'…'` (ANSI-C quoting — the `$` is read as a literal and the quote after it as an ordinary
// single quote, so a word built with it is misread), `$"…"` (locale translation), brace expansion
// (`{a,b}`), and a `case` pattern's unbalanced `)` inside `$(…)`, whose extent is found by
// parenthesis balance. The one shape that lets content past a quote read as flags again is
// genuinely unterminated input (a quote with no matching close at all), which a real shell also
// refuses to run, so it is not a bypass this guard's decision on it can actually affect.
//
// An internal error fails OPEN for every command except one that plausibly names a git commit or
// push — --no-verify and the unarmed-repository check are the only two cases this guard uniquely
// covers (the git hooks do not exist to catch them), so silently allowing on this script's own
// malfunction would leave nothing enforcing the gate for exactly those two cases. Every other
// command fails open: a bug in this script must never brick a session.

import process from "node:process";
import { runGit } from "../../scripts/lib/run-git.mjs";

const ENV_ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;
// The configuration-carrying environment variables git(1) documents, matched by NAME only and
// case-insensitively (see the header's ENVIRONMENT rule).
const GIT_CONFIG_ENV_NAME = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)$/i;
const GIT_CONFIG_ENV_RE = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)=/i;
// Matches a bare `export` argument naming a GIT_CONFIG_* variable, with or without a `=value` —
// `export GIT_CONFIG_KEY_0` (exporting an already-assigned shell variable) redirects the gate
// exactly like `export GIT_CONFIG_KEY_0=...` (assigning and exporting in one step); only the
// value's presence differs, never whether the override applies.
const GIT_CONFIG_EXPORT_ARG_RE = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)(=.*)?$/i;

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
 * Recognition, Redirection, Command Substitution, Here-Document). The rules, each of which the
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
 *           segment.
 *   Redirection — An unquoted `<` or `>` ends the word before it and begins a redirection
 *           operator (`<`, `<&`, `<>`, `>`, `>>`, `>&`, `>|`, and bash's `&>`, `&>>`); the
 *           operator and the word after it (its target, possibly quoted) are removed from the
 *           command's arguments wherever they sit, so `-m >out x` still gives `-m` the value
 *           `x`, and `--no-verify>out` is still the flag. An unquoted word of only digits
 *           directly before the operator is its file-descriptor number and is removed with it
 *           (`2>&1`). `<<<` is a here-string: its operand word is data, removed the same way.
 *   Here-Document — `<<` or `<<-` followed by a delimiter word (quoted or not) opens a here-document
 *           whose body starts after the next unquoted <newline> and ends at the first line equal
 *           to the delimiter (leading tabs stripped under `<<-`); with an unquoted delimiter a
 *           body line ending in backslash joins the next line before the comparison. The body is
 *           data, never a segment.
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
  // True while the current word consists solely of unquoted digits (a candidate IO_NUMBER).
  let curPlainDigits = false;
  // True when the next completed word is a redirection target, removed rather than kept.
  let dropNextWord = false;

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

  const add = (s, plainDigit = false) => {
    curPlainDigits = (has ? curPlainDigits : true) && plainDigit;
    cur += s;
    has = true;
  };
  const flushToken = () => {
    if (has) {
      if (dropNextWord) dropNextWord = false;
      else tokens.push(cur);
      cur = "";
      has = false;
      curPlainDigits = false;
    }
  };
  const flushSegment = () => {
    flushToken();
    dropNextWord = false;
    if (tokens.length) segments.push(tokens);
    tokens = [];
  };

  // True at a position where an unquoted `#` starts a comment: the start of a word.
  let atWordStart = true;
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
      continue;
    }
    if (ch === "'" || ch === '"') {
      push({ kind: ch === "'" ? "single" : "double" });
      if (verbatim) add(ch);
      else {
        has = true; // an empty quoted span is still a (possibly empty) word
        curPlainDigits = false;
      }
      atWordStart = false;
      i++;
      continue;
    }
    if (ch === "`") {
      push({ kind: "backtick" });
      add(ch);
      atWordStart = false;
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
    if (ch === "<" && text[i + 1] === "<" && text[i + 2] !== "<") {
      const j = readHeredocOperator(i);
      if (verbatim) add(text.slice(i, j));
      else flushToken();
      atWordStart = true;
      i = j;
      continue;
    }
    if (verbatim) {
      add(ch);
      atWordStart = HEREDOC_DELIMITER_END.test(ch);
      i++;
      continue;
    }
    if (ch === "<" || ch === ">" || (ch === "&" && text[i + 1] === ">")) {
      // Redirection rule: the operator and its target word leave the argument list; an
      // unquoted all-digit word directly before it is the descriptor number and leaves too —
      // unless that word is itself the target of the operator before it (`> 2>&1` writes to a
      // file named `2`), in which case it is dropped as a target.
      if (has && curPlainDigits && !dropNextWord) {
        cur = "";
        has = false;
        curPlainDigits = false;
      } else flushToken();
      let j = i + 1;
      if (ch === "&") {
        j++; // `&>`
        if (text[j] === ">") j++; // `&>>`
      } else if (ch === ">") {
        if (text[j] === ">" || text[j] === "|" || text[j] === "&") j++;
      } else if (text[j] === "<" && text[j + 1] === "<") {
        j += 2; // `<<<` here-string
      } else if (text[j] === "&" || text[j] === ">") {
        j++;
      }
      dropNextWord = true;
      atWordStart = true;
      i = j;
      continue;
    }
    if (ch === ";" || ch === "&" || ch === "|") {
      flushSegment();
      while (i + 1 < n && ";&|".includes(text[i + 1])) i++;
      atWordStart = true;
      i++;
      continue;
    }
    if (/\s/.test(ch)) {
      flushToken();
      atWordStart = true;
      i++;
      continue;
    }
    add(ch, /\d/.test(ch));
    atWordStart = false;
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

// ---------------------------------------------------------------------------------------------
// git(1) OPTIONS rule: the front end's own options, matched by exact string. These take their
// value as the NEXT word (`--config-env` and `--attr-source` also accept `--name=value`; `-c`
// and `-C` accept only the separate form — `-ccore.x=1` is an unknown option to git). Every
// other option is a flag, and any `--name=value` is one word. Parsing ends at the first word
// that is not an option: the subcommand.
// ---------------------------------------------------------------------------------------------
const MAIN_SEPARATE_VALUE_OPTIONS = new Set([
  "-C",
  "-c",
  "--config-env",
  "--git-dir",
  "--work-tree",
  "--namespace",
  "--attr-source",
]);

/** Index in `gitArgs` (whose element 0 is the git word) of the subcommand, or -1 when there is none. */
function subcommandIndex(gitArgs) {
  for (let i = 1; i < gitArgs.length; i++) {
    const t = gitArgs[i];
    if (MAIN_SEPARATE_VALUE_OPTIONS.has(t)) {
      i++;
      continue;
    }
    if (t.startsWith("-")) continue;
    return i;
  }
  return -1;
}

// ---------------------------------------------------------------------------------------------
// git-config(1) name syntax: `section.variable` or `section.subsection.variable`; section and
// variable names fold case, the subsection (everything between the first and last dot) does not.
// ---------------------------------------------------------------------------------------------

/** `key` in git's canonical spelling: section and variable lowercased, subsection untouched. */
function foldConfigKey(key) {
  const first = key.indexOf(".");
  if (first === -1) return key.toLowerCase();
  const last = key.lastIndexOf(".");
  if (last === first) return key.toLowerCase();
  return key.slice(0, first).toLowerCase() + key.slice(first, last + 1) + key.slice(last + 1).toLowerCase();
}

/** The folded section of a key (everything before the last dot) — for section-level operations. */
function sectionOf(foldedKey) {
  const last = foldedKey.lastIndexOf(".");
  return last === -1 ? foldedKey : foldedKey.slice(0, last);
}

const GATE_KEYS = new Set(["core.hookspath", "extensions.worktreeconfig"]);
const GATE_SECTIONS = new Set(["core", "extensions"]);

/** True for `include.path` and `includeIf.<condition>.path`, the config include directives. */
function isIncludeDirective(foldedKey) {
  return foldedKey === "include.path" || (foldedKey.startsWith("includeif.") && foldedKey.endsWith(".path"));
}

/**
 * Every configuration pair this invocation adds at command-line precedence, in order:
 * `-c name[=value]`, `--config-env name=ENVVAR` (value read from a leading `ENVVAR=value`
 * assignment in the same segment when there is one, else unknown), and — from the segment's
 * leading assignments — `GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` pairs and the words of
 * `GIT_CONFIG_PARAMETERS` (each a shell-quoted `'key'='value'`, split by the shell rules).
 * Keys are returned folded; `value` is `null` when it cannot be read from the command string.
 *
 * @returns {{ key: string, value: string | null }[]}
 */
function inlineConfigPairs(tokens, gi) {
  const pairs = [];
  const assigned = new Map();
  const envKeys = new Map();
  const envValues = new Map();
  for (let i = 0; i < gi; i++) {
    const eq = tokens[i].indexOf("=");
    const name = tokens[i].slice(0, eq);
    const value = tokens[i].slice(eq + 1);
    assigned.set(name, value);
    const upper = name.toUpperCase();
    if (!GIT_CONFIG_ENV_NAME.test(upper)) continue;
    if (upper.startsWith("GIT_CONFIG_KEY_")) envKeys.set(upper.slice("GIT_CONFIG_KEY_".length), value);
    else if (upper.startsWith("GIT_CONFIG_VALUE_")) envValues.set(upper.slice("GIT_CONFIG_VALUE_".length), value);
    else if (upper === "GIT_CONFIG_PARAMETERS") {
      for (const word of segmentAndTokenize(value).flat()) {
        const weq = word.indexOf("=");
        pairs.push(
          weq === -1
            ? { key: foldConfigKey(word), value: null }
            : { key: foldConfigKey(word.slice(0, weq)), value: word.slice(weq + 1) },
        );
      }
    }
  }
  for (const [index, key] of envKeys) {
    pairs.push({ key: foldConfigKey(key), value: envValues.has(index) ? envValues.get(index) : null });
  }
  const end = subcommandIndex(tokens.slice(gi));
  const stop = end === -1 ? tokens.length : gi + end;
  for (let i = gi + 1; i < stop; i++) {
    const t = tokens[i];
    let spec = null;
    let fromEnv = false;
    if (t === "-c") spec = tokens[++i];
    else if (t === "--config-env") {
      spec = tokens[++i];
      fromEnv = true;
    } else if (t.startsWith("--config-env=")) {
      spec = t.slice("--config-env=".length);
      fromEnv = true;
    } else continue;
    if (spec === undefined) break;
    const eq = spec.indexOf("=");
    const key = foldConfigKey(eq === -1 ? spec : spec.slice(0, eq));
    if (!fromEnv) pairs.push({ key, value: eq === -1 ? null : spec.slice(eq + 1) });
    else {
      const envName = eq === -1 ? null : spec.slice(eq + 1);
      pairs.push({ key, value: envName !== null && assigned.has(envName) ? assigned.get(envName) : null });
    }
  }
  return pairs;
}

// ---------------------------------------------------------------------------------------------
// parse-options API rule: per-subcommand option tables, taken from `git <cmd> -h` (the test suite
// re-derives them from the installed git and fails on drift). `arg` is "none", "required"
// (`--name value`/`--name=value`, `-x value`/`-xvalue`) or "optional" (`--name=value`, `-xvalue`
// only). `noneg` marks the options git prints without `[no-]`.
// ---------------------------------------------------------------------------------------------

/** @typedef {{ long: string, short?: string, arg: "none" | "required" | "optional", noneg?: boolean }} OptionSpec */

/** @param {OptionSpec[]} specs */
function optionTable(specs) {
  const byShort = new Map();
  for (const spec of specs) if (spec.short) byShort.set(spec.short, spec);
  return { specs, byShort };
}

const COMMIT_OPTIONS = optionTable([
  { long: "quiet", short: "q", arg: "none" },
  { long: "verbose", short: "v", arg: "none" },
  { long: "file", short: "F", arg: "required" },
  { long: "author", arg: "required" },
  { long: "date", arg: "required" },
  { long: "message", short: "m", arg: "required" },
  { long: "reedit-message", short: "c", arg: "required" },
  { long: "reuse-message", short: "C", arg: "required" },
  { long: "fixup", arg: "required" },
  { long: "squash", arg: "required" },
  { long: "reset-author", arg: "none" },
  { long: "trailer", arg: "required", noneg: true },
  { long: "signoff", short: "s", arg: "none" },
  { long: "template", short: "t", arg: "required" },
  { long: "edit", short: "e", arg: "none" },
  { long: "cleanup", arg: "required" },
  { long: "status", arg: "none" },
  { long: "gpg-sign", short: "S", arg: "optional" },
  { long: "all", short: "a", arg: "none" },
  { long: "include", short: "i", arg: "none" },
  { long: "interactive", arg: "none" },
  { long: "patch", short: "p", arg: "none" },
  { long: "unified", short: "U", arg: "required", noneg: true },
  { long: "inter-hunk-context", arg: "required", noneg: true },
  { long: "only", short: "o", arg: "none" },
  { long: "no-verify", short: "n", arg: "none" },
  { long: "dry-run", arg: "none" },
  { long: "short", arg: "none" },
  { long: "branch", arg: "none" },
  { long: "ahead-behind", arg: "none" },
  { long: "porcelain", arg: "none" },
  { long: "long", arg: "none" },
  { long: "null", short: "z", arg: "none" },
  { long: "amend", arg: "none" },
  { long: "no-post-rewrite", arg: "none" },
  { long: "untracked-files", short: "u", arg: "optional" },
  { long: "pathspec-from-file", arg: "required" },
  { long: "pathspec-file-nul", arg: "none" },
]);

const PUSH_OPTIONS = optionTable([
  { long: "verbose", short: "v", arg: "none" },
  { long: "quiet", short: "q", arg: "none" },
  { long: "repo", arg: "required" },
  { long: "all", arg: "none" },
  { long: "branches", arg: "none" },
  { long: "mirror", arg: "none" },
  { long: "delete", short: "d", arg: "none" },
  { long: "tags", arg: "none" },
  { long: "dry-run", short: "n", arg: "none" },
  { long: "porcelain", arg: "none" },
  { long: "force", short: "f", arg: "none" },
  { long: "force-with-lease", arg: "optional" },
  { long: "force-if-includes", arg: "none" },
  { long: "recurse-submodules", arg: "required" },
  { long: "thin", arg: "none" },
  { long: "receive-pack", arg: "required" },
  { long: "exec", arg: "required" },
  { long: "set-upstream", short: "u", arg: "none" },
  { long: "progress", arg: "none" },
  { long: "prune", arg: "none" },
  { long: "no-verify", arg: "none" },
  { long: "follow-tags", arg: "none" },
  { long: "signed", arg: "optional" },
  { long: "atomic", arg: "none" },
  { long: "push-option", short: "o", arg: "required" },
  { long: "ipv4", short: "4", arg: "none", noneg: true },
  { long: "ipv6", short: "6", arg: "none", noneg: true },
]);

// git-config(1): the subcommand options and the deprecated flag modes, as one table — a legacy
// mode flag and a subcommand option never collide by name.
const CONFIG_OPTIONS = optionTable([
  { long: "global", arg: "none" },
  { long: "system", arg: "none" },
  { long: "local", arg: "none" },
  { long: "worktree", arg: "none" },
  { long: "file", short: "f", arg: "required" },
  { long: "blob", arg: "required" },
  { long: "type", short: "t", arg: "required" },
  { long: "bool", arg: "none" },
  { long: "int", arg: "none" },
  { long: "bool-or-int", arg: "none" },
  { long: "bool-or-str", arg: "none" },
  { long: "path", arg: "none" },
  { long: "expiry-date", arg: "none" },
  { long: "null", short: "z", arg: "none" },
  { long: "name-only", arg: "none" },
  { long: "show-origin", arg: "none" },
  { long: "show-scope", arg: "none" },
  { long: "show-names", arg: "none" },
  { long: "includes", arg: "none" },
  { long: "default", arg: "required" },
  { long: "comment", arg: "required" },
  { long: "all", arg: "none" },
  { long: "regexp", arg: "none" },
  { long: "value", arg: "required" },
  { long: "url", arg: "required" },
  { long: "fixed-value", arg: "none" },
  { long: "append", arg: "none" },
  { long: "replace-all", arg: "none" },
  { long: "add", arg: "none" },
  { long: "unset", arg: "none" },
  { long: "unset-all", arg: "none" },
  { long: "rename-section", arg: "none" },
  { long: "remove-section", arg: "none" },
  { long: "edit", short: "e", arg: "none" },
  { long: "list", short: "l", arg: "none" },
  { long: "get", arg: "none" },
  { long: "get-all", arg: "none" },
  { long: "get-regexp", arg: "none" },
  { long: "get-urlmatch", arg: "none" },
  { long: "get-color", arg: "none" },
  { long: "get-colorbool", arg: "none" },
]);

/** The tables the test suite pins against `git <cmd> -h`; keyed by subcommand. */
export const OPTION_TABLES = { commit: COMMIT_OPTIONS, push: PUSH_OPTIONS, config: CONFIG_OPTIONS };

/**
 * Resolves a long option's name (the text between `--` and any `=`) the way parse-options does:
 * every option is spelled `name`; unless `noneg`, also `no-name` (negated) and, for an option
 * itself named `no-x`, `x` (negated). An exact spelling wins; otherwise the name must be a
 * prefix of exactly one option's spellings. Returns `{ spec, negated }`, `{ ambiguous: [specs] }`
 * for a prefix naming several options (an error in git), or `null` for an unknown option.
 */
function resolveLongOption(name, table) {
  if (!name) return null;
  const candidates = new Map();
  for (const spec of table.specs) {
    const spellings = [{ spelling: spec.long, negated: false }];
    if (!spec.noneg) {
      spellings.push({ spelling: `no-${spec.long}`, negated: true });
      if (spec.long.startsWith("no-")) spellings.push({ spelling: spec.long.slice(3), negated: true });
    }
    for (const { spelling, negated } of spellings) {
      if (spelling === name) return { spec, negated };
      if (spelling.startsWith(name) && !candidates.has(spec)) candidates.set(spec, negated);
    }
  }
  if (candidates.size === 1) {
    const [[spec, negated]] = candidates;
    return { spec, negated };
  }
  if (candidates.size > 1) return { ambiguous: [...candidates.keys()] };
  return null;
}

/**
 * Walks a subcommand's arguments under the parse-options rules (see the table comment) and
 * returns every option occurrence and every operand. `options[i].spec` is `null` for an option
 * git would reject as unknown, and `ambiguous` lists the candidates for a prefix naming several.
 *
 * @returns {{ options: { spec: OptionSpec | null, negated: boolean, ambiguous?: OptionSpec[] }[], operands: string[] }}
 */
function walkOptions(args, table) {
  const options = [];
  const operands = [];
  for (let i = 0; i < args.length; i++) {
    const t = args[i];
    if (t === "--") {
      operands.push(...args.slice(i + 1));
      break;
    }
    if (t.startsWith("--")) {
      const eq = t.indexOf("=");
      const name = eq === -1 ? t.slice(2) : t.slice(2, eq);
      const resolved = resolveLongOption(name, table);
      if (!resolved) options.push({ spec: null, negated: false });
      else if (resolved.ambiguous) options.push({ spec: null, negated: false, ambiguous: resolved.ambiguous });
      else {
        options.push(resolved);
        if (!resolved.negated && resolved.spec.arg === "required" && eq === -1) i++;
      }
      continue;
    }
    if (t.startsWith("-") && t.length > 1) {
      for (let j = 1; j < t.length; j++) {
        const spec = table.byShort.get(t[j]) ?? null;
        options.push({ spec, negated: false });
        if (spec && spec.arg !== "none") {
          if (spec.arg === "required" && j === t.length - 1) i++;
          break; // the rest of the bundle (or the next word) is this option's value
        }
      }
      continue;
    }
    operands.push(t);
  }
  return { options, operands };
}

/** True when any option in `walk` is a positive `no-verify`, or a prefix git would reject as ambiguous with it. */
function hasNoVerify(walk) {
  return walk.options.some(
    (o) =>
      (o.spec?.long === "no-verify" && !o.negated) ||
      (o.ambiguous?.some((spec) => spec.long === "no-verify") ?? false),
  );
}

// git-config(1) modes. The subcommand forms are the first operand; the deprecated forms are
// flags; with neither, one operand reads and two or more write.
const CONFIG_SUBCOMMANDS = new Set(["list", "get", "set", "unset", "rename-section", "remove-section", "edit"]);
const CONFIG_READ_MODES = new Set(["list", "get", "get-all", "get-regexp", "get-urlmatch", "get-color", "get-colorbool"]);
const CONFIG_WRITE_MODES = new Set(["set", "add", "replace-all", "unset", "unset-all", "rename-section", "remove-section", "edit"]);
const CONFIG_SECTION_MODES = new Set(["rename-section", "remove-section"]);

/**
 * True when `git config <configArgs>` writes to a gate key or a gate section, in any spelling.
 * Every operand is checked rather than only the operand in name position, so an option this
 * table does not know (whose value would then read as an extra operand) cannot shift the name
 * out of view.
 */
function configWritesGate(configArgs) {
  const walk = walkOptions(configArgs, CONFIG_OPTIONS);
  let mode = null;
  let operands = walk.operands;
  if (operands.length && CONFIG_SUBCOMMANDS.has(operands[0])) {
    mode = operands[0];
    operands = operands.slice(1);
  } else {
    // git refuses more than one mode flag per invocation, so when several are present no write
    // happens; taking a write mode over a read mode here keeps that refusal on the safe side
    // (`-le` is read as `--edit`, never as `--list`).
    const modes = walk.options.filter((o) => !o.negated && o.spec).map((o) => o.spec.long);
    mode =
      modes.find((name) => CONFIG_WRITE_MODES.has(name)) ??
      modes.find((name) => CONFIG_READ_MODES.has(name)) ??
      (operands.length >= 2 ? "set" : "get");
  }
  if (!CONFIG_WRITE_MODES.has(mode)) return false;
  if (mode === "edit") return true;
  const folded = operands.map(foldConfigKey);
  if (CONFIG_SECTION_MODES.has(mode)) return folded.some((k) => GATE_SECTIONS.has(sectionOf(k)) || GATE_SECTIONS.has(k));
  return folded.some((k) => GATE_KEYS.has(k));
}

/**
 * True when a leading `NAME=value` assignment in THIS segment, immediately before the git word at
 * `gi`, sets a configuration-carrying `GIT_CONFIG_*` variable (see the header's ENVIRONMENT
 * rule) — it redirects core.hooksPath exactly like `-c core.hooksPath=...` without that string
 * ever appearing as a `-c` token. Scoped to the same segment as `gitTokenIndex` already scopes
 * the assignments themselves: a bare `VAR=value` prefix (with no `export`) applies only to the
 * command it directly prefixes, never to a later command in a `;`/`&&` chain, so checking other
 * segments would deny an unrelated command that merely follows one — unlike `export`, below,
 * which genuinely does propagate.
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
const INCLUDE_REFUSAL =
  "Refused: an inline include.path / includeIf.*.path pulls a configuration file in at command-line precedence, where it can carry core.hooksPath. If the gate itself is wrong, stop and report it to the owner.";
const GIT_CONFIG_ENV_REFUSAL =
  "Refused: a GIT_CONFIG_* environment variable can redirect core.hooksPath for a single git invocation without the string ever appearing as a -c token. Setting any GIT_CONFIG_* variable around a commit or push is refused. If the gate itself is wrong, stop and report it to the owner.";
const NO_VERIFY_REFUSAL =
  "Refused: --no-verify (or -n on commit) skips the local gate. There is no bypass in this project. If the gate cannot pass, stop and report it to the owner.";
const UNARMED_REFUSAL =
  "Refused: this repository has no gate installed (core.hooksPath is unset). Run `pnpm install` to arm it before committing or pushing.";
const ALIAS_REFUSAL =
  "Refused: the invoked subcommand is an alias defined inline for this invocation whose expansion this guard cannot read. Invoke the git command directly.";

// Alias-to-alias chains beyond this depth are refused rather than followed.
const MAX_ALIAS_DEPTH = 10;

const deny = (reason) => ({ deny: true, reason });
const ALLOW = { deny: false, reason: "" };

/** Quotes `word` for a POSIX shell, so a `!`-alias expansion can be rebuilt as the string git would run. */
function shellQuote(word) {
  return `'${word.replace(/'/g, "'\\''")}'`;
}

/**
 * Classifies one segment whose command word (at `gi`) is git, expanding an inline alias in
 * place of its subcommand until the subcommand is one this guard reads (`commit`, `push`,
 * `config`) or is not aliased. `context` carries the enclosing segments (for `export`
 * propagation), the armed accessor, and the alias depth.
 */
function classifyGitSegment(tokens, gi, context) {
  const pairs = inlineConfigPairs(tokens, gi);
  for (const { key } of pairs) {
    if (GATE_KEYS.has(key)) return deny(GATE_CONFIG_REFUSAL);
    if (isIncludeDirective(key)) return deny(INCLUDE_REFUSAL);
  }

  const gitArgs = tokens.slice(gi);
  const si = subcommandIndex(gitArgs);
  if (si === -1) return ALLOW;
  const sub = gitArgs[si];
  const subArgs = gitArgs.slice(si + 1);

  if (sub === "config") return configWritesGate(subArgs) ? deny(GATE_CONFIG_REFUSAL) : ALLOW;

  if (sub === "commit" || sub === "push") {
    const envOverride = hasGitConfigEnvOverride(tokens, gi) || context.priorSegments.some(segmentExportsGitConfigVar);
    if (envOverride) return deny(GIT_CONFIG_ENV_REFUSAL);
    if (hasNoVerify(walkOptions(subArgs, sub === "commit" ? COMMIT_OPTIONS : PUSH_OPTIONS))) {
      return deny(NO_VERIFY_REFUSAL);
    }
    if (!context.isArmed()) return deny(UNARMED_REFUSAL);
    return ALLOW;
  }

  // git-config(1) `alias.<name>`: an alias defined inline for the subcommand being invoked.
  const aliasKey = `alias.${sub.toLowerCase()}`;
  const alias = pairs.filter((p) => p.key === aliasKey).pop();
  if (!alias) return ALLOW;
  if (alias.value === null) return deny(ALIAS_REFUSAL);
  if (context.aliasDepth >= MAX_ALIAS_DEPTH) return deny(ALIAS_REFUSAL);
  const next = { ...context, aliasDepth: context.aliasDepth + 1 };
  if (alias.value.startsWith("!")) {
    const shell = [alias.value.slice(1), ...subArgs.map(shellQuote)].join(" ");
    return classifyCommand(shell, next);
  }
  const expansion = segmentAndTokenize(alias.value).flat();
  const expanded = [...tokens.slice(0, gi + si), ...expansion, ...subArgs];
  return classifyGitSegment(expanded, gi, next);
}

/** Classifies every segment of a shell command string under `context`. */
function classifyCommand(command, context) {
  const segments = segmentAndTokenize(command);
  for (let segIdx = 0; segIdx < segments.length; segIdx++) {
    const tokens = segments[segIdx];
    const gi = gitTokenIndex(tokens);
    if (gi === -1) continue;
    const verdict = classifyGitSegment(tokens, gi, {
      ...context,
      priorSegments: [...context.priorSegments, ...segments.slice(0, segIdx)],
    });
    if (verdict.deny) return verdict;
  }
  return ALLOW;
}

/** Whether this command must be refused, and what to tell the agent. */
export function classify(command, { hooksPathSet }) {
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
  return classifyCommand(command, { isArmed, priorSegments: [], aliasDepth: 0 });
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
      // Registered for every tool: a payload with no command string (an edit, a read) is a no-op.
      const field = JSON.parse(raw)?.tool_input?.command;
      command = typeof field === "string" ? field : "";
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
