import { test, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { execPath } from "node:process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import {
  classify,
  segmentAndTokenize,
  looksLikeCommitOrPush,
  bestEffortCommand,
} from "../.claude/hooks/guard-git.mjs";

const armed = { hooksPathSet: true };
const bare = { hooksPathSet: false };
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

// ---------------------------------------------------------------------------------------------
// The tokenizer implements the POSIX Shell Command Language quoting and token-recognition rules
// (IEEE Std 1003.1-2017 XCU chapter 2: Quoting, Token Recognition, Command Substitution, Here-Document) as a grammar. Every case below is derived
// from one rule of that grammar — never from a particular reported command — and each rule that
// can deny is paired with the innocent twin that must stay allowed, because a false denial here
// blocks real work and has no off switch. Where the exact token shape is the rule's whole content
// (a continuation inserting NOTHING, a heredoc body vanishing), `segmentAndTokenize` is asserted
// directly so the assertion cannot pass by a coincidence of the verdict.
// ---------------------------------------------------------------------------------------------

// Escape Character rule — outside quotes, backslash-<newline> is a line continuation: both characters are
// removed and nothing is inserted, so the surrounding text joins into one word.
test("Escape Character rule: backslash-newline outside quotes joins the surrounding text with nothing inserted", () => {
  expect(segmentAndTokenize("git commit --no\\\n-verify -m x")).toEqual([
    ["git", "commit", "--no-verify", "-m", "x"],
  ]);
  expect(classify("git commit --no\\\n-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit -m x \\\n--no-verify", armed).deny).toBe(true);
  expect(classify("git -c core.hooks\\\nPath=/x commit -m x", armed).deny).toBe(true);
  expect(classify("ex\\\nport GIT_CONFIG_COUNT=1 && git commit -m x", armed).deny).toBe(true);
  // Innocent twins: a continuation between ordinary arguments, and one that splits the git word
  // itself — which must still be recognised as a commit (the unarmed check proves it is).
  expect(classify("git commit \\\n-m x", armed).deny).toBe(false);
  expect(classify("git \\\ncommit -m x", armed).deny).toBe(false);
  expect(classify("git \\\ncommit -m x", bare).deny).toBe(true);
  // A continuation is not a separator either: `x\<newline>--no-verify` is the single word
  // `x--no-verify`, the message, not a flag.
  expect(segmentAndTokenize("git commit -m x\\\n--no-verify")).toEqual([
    ["git", "commit", "-m", "x--no-verify"],
  ]);
  expect(classify("git commit -m x\\\n--no-verify", armed).deny).toBe(false);
});

// Escape Character rule — outside quotes, a backslash preserves the literal value of the next character: a
// quote, whitespace, or an operator character.
test("Escape Character rule: a backslash outside quotes escapes a quote, whitespace, or a separator", () => {
  expect(segmentAndTokenize('a \\"b \\&\\& c\\;d e\\ f')).toEqual([["a", '"b', "&&", "c;d", "e f"]]);
  expect(classify('git commit -m \\"x --no-verify', armed).deny).toBe(true);
  expect(classify("git commit -m fix\\ this --no-verify", armed).deny).toBe(true);
  // An escaped `&&` is an ARGUMENT to git, so the flag after it belongs to the same command.
  expect(classify("git commit -m x \\&\\& git commit --no-verify", armed).deny).toBe(true);
  // Innocent twins: the escaped quote is just a message character, and an escaped `;` keeps the
  // bypass text inside the -m value.
  expect(classify('git commit -m \\"x\\"', armed).deny).toBe(false);
  expect(classify("git commit -m x\\;--no-verify", armed).deny).toBe(false);
  expect(classify("git commit -m fix\\ this", armed).deny).toBe(false);
  // A trailing backslash with nothing to escape stays literal (bash passes `a\` through).
  expect(segmentAndTokenize("a\\")).toEqual([["a\\"]]);
});

// Single-Quotes rule — inside single quotes every character is literal: backslash, double quote, `$`,
// backquote, and newline included; backslash-<newline> is NOT a continuation there.
test("Single-Quotes rule: single quotes make every character literal, including backslash-newline", () => {
  expect(segmentAndTokenize("'a\\\nb' '\"$x`'")).toEqual([["a\\\nb", '"$x`']]);
  expect(classify("git commit '--no-verify' -m x", armed).deny).toBe(true);
  expect(classify("git commit -m 'a\\\nb' --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m 'raw \\n text' --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m '$(x) \"y\"' --no-verify", armed).deny).toBe(true);
  // Innocent twins.
  expect(classify("git commit -m 'a; git commit --no-verify'", armed).deny).toBe(false);
  expect(classify("git commit -m 'a\\\nb'", armed).deny).toBe(false);
  expect(classify("git commit -m 'raw \\n text'", armed).deny).toBe(false);
  expect(classify("git commit -m '$(x) \"y\"'", armed).deny).toBe(false);
});

// Double-Quotes rule — inside double quotes a backslash escapes exactly `"`, `\`, `$`, backquote and
// <newline> (the last as a continuation, removed); before any other character it is literal.
test("Double-Quotes rule: the five double-quote escapes, and a literal backslash before anything else", () => {
  expect(segmentAndTokenize('"\\"\\\\\\$\\`" "a\\\nb" "a\\nb"')).toEqual([['"\\$`', "ab", "a\\nb"]]);
  expect(classify('git commit -m "fix: escape \\" test" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "a \\" b \\" c" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "path\\\\dir" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "cost \\$5" --no-verify', armed).deny).toBe(true);
  expect(classify("git commit -m \"run \\`x\\`\" --no-verify", armed).deny).toBe(true);
  expect(classify('git commit "--no\\\n-verify" -m x', armed).deny).toBe(true);
  expect(classify('git -c "core.hooks\\\nPath=/x" commit -m x', armed).deny).toBe(true);
  expect(classify('git commit -m "path\\ndir" --no-verify', armed).deny).toBe(true);
  // Innocent twins.
  expect(classify('git commit -m "fix: escape \\" test"', armed).deny).toBe(false);
  expect(classify('git commit -m "a\\\nb"', armed).deny).toBe(false);
  expect(classify('git commit -m "path\\ndir"', armed).deny).toBe(false);
  expect(classify("git commit -m \"it's fine\"", armed).deny).toBe(false);
  expect(classify("git commit -m \"it's fine\" --no-verify", armed).deny).toBe(true);
});

// Double-Quotes rule — a newline inside double quotes is literal content, not a segment boundary.
test("Double-Quotes rule: a newline inside double quotes is message content, not a separator", () => {
  expect(classify('git commit -m "subject\n\ngit commit --no-verify is refused"', armed).deny).toBe(
    false,
  );
  expect(classify('git commit -m "subject\n\nbody" --no-verify', armed).deny).toBe(true);
});

// Double-Quotes rule — `${…}` inside double quotes carries its own balanced quoting; `$(…)`, `$((…))` and a
// backquoted substitution are parsed by the grammar in their own right (the Command Substitution rule). A double quote
// inside any of them never closes the enclosing span, and the substitution's text is kept
// verbatim as opaque content (never expanded).
test("Double-Quotes and Command Substitution rules: a substitution inside double quotes keeps its own quotes without closing the outer span", () => {
  expect(segmentAndTokenize('"${M:-"a b"}" "$(printf "%s" "a b")" "`printf "%s" "a b"`" "$((1+(2*3)))" t')).toEqual([
    ['${M:-"a b"}', '$(printf "%s" "a b")', '`printf "%s" "a b"`', "$((1+(2*3)))", "t"],
  ]);
  // Innocent: a `-n` inside the nested quotes is message content.
  expect(classify('git commit -m "${MSG:-"x -n "}"', armed).deny).toBe(false);
  expect(classify('git commit -m "$(printf "%s" "x -n ")"', armed).deny).toBe(false);
  expect(classify('git commit -m "`printf "%s" "x -n "`"', armed).deny).toBe(false);
  expect(classify('git commit -m "$((1+(2*3))) -n "', armed).deny).toBe(false);
  // Denied: the flag after the closing quote is a real flag.
  expect(classify('git commit -m "${MSG:-"x"}" -n', armed).deny).toBe(true);
  expect(classify('git commit -m "$(printf "%s" "x")" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "`date`" --no-verify', armed).deny).toBe(true);
  expect(classify('git commit -m "$((1+(2*3)))" --no-verify', armed).deny).toBe(true);
  // Unquoted substitutions are opaque words too.
  expect(segmentAndTokenize("a $(b c) ${d e} `f g` h")).toEqual([["a", "$(b c)", "${d e}", "`f g`", "h"]]);
  expect(classify("git commit -m $(printf x) --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m $(printf x)", armed).deny).toBe(false);
});

// Token Recognition rule — adjacent quoted and unquoted parts form one word; empty quotes form an empty word.
test("Token Recognition rule: adjacent quoted and unquoted parts join into one word", () => {
  expect(segmentAndTokenize("--no\"\"-verify \"--no\"-verify --no-'verify' \"\"")).toEqual([
    ["--no-verify", "--no-verify", "--no-verify", ""],
  ]);
  expect(classify('git commit --no""-verify -m x', armed).deny).toBe(true);
  expect(classify('git commit "--no"-verify -m x', armed).deny).toBe(true);
  expect(classify("git commit --no-'verify' -m x", armed).deny).toBe(true);
  expect(classify('git commit "--no-verify" -m x', armed).deny).toBe(true);
  expect(classify('git commit --no-verify"" -m x', armed).deny).toBe(true);
  // Innocent twins: an empty message word, and a joined word that is NOT the flag.
  expect(classify('git commit -m ""', armed).deny).toBe(false);
  expect(classify('git commit -m x --no-"verify me"', armed).deny).toBe(false);
});

// Token Recognition rule — an unquoted `#` at the start of a word begins a comment running to the newline. The
// comment is discarded whole: a quote character inside it opens nothing, and a backslash-<newline>
// inside it is not a continuation. A `#` inside a word is an ordinary character.
test("Token Recognition rule: a comment is discarded to the newline and nothing inside it has any effect", () => {
  expect(segmentAndTokenize('a b # c "d\ne # \\\nf b#c')).toEqual([["a", "b"], ["e"], ["f", "b#c"]]);
  // Innocent: the bypass text is inside a comment, or inside a word.
  expect(classify("git commit -m x # --no-verify", armed).deny).toBe(false);
  expect(classify("# git commit --no-verify\ngit commit -m x", armed).deny).toBe(false);
  expect(classify("git commit -m x#--no-verify", armed).deny).toBe(false);
  // Denied: the line after a comment is a real command, whatever the comment contained.
  expect(classify('echo hi #"\ngit commit --no-verify -m x', armed).deny).toBe(true);
  expect(classify("echo hi # \\\ngit commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit --no-verify -m x # comment", armed).deny).toBe(true);
});

// Token Recognition rule — `>&`, `<&`, `>|` (and bash's `&>`, `&>>`) are redirection operators: the `&`/`|` in
// them does not end the segment, so the arguments after them still belong to the same command.
test("Token Recognition rule: a redirection operator containing & or | is not a chain separator", () => {
  expect(segmentAndTokenize("a 2>&1 b >|f c &>g d <&0 e")).toEqual([
    ["a", "2>&1", "b", ">|f", "c", "&>g", "d", "<&0", "e"],
  ]);
  expect(classify("git commit -m x 2>&1 --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x >|out --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x &>/dev/null --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x <&0 --no-verify", armed).deny).toBe(true);
  // Innocent twins, and a real `&` after a redirection still separates.
  expect(classify("git commit -m x 2>&1", armed).deny).toBe(false);
  expect(classify("git commit -m x 2>&1 | tail -1", armed).deny).toBe(false);
  expect(segmentAndTokenize("git push 2>&1 & git status")).toEqual([["git", "push", "2>&1"], ["git", "status"]]);
  expect(classify("git push 2>&1 & git status", armed).deny).toBe(false);
});

// Here-Document rule — a here-document body starts after the next unquoted newline and ends at the first line
// equal to the delimiter (leading tabs stripped under `<<-`). The body is data: nothing in it is
// a command, and with an unquoted delimiter a body line ending in backslash joins the next line
// before the comparison. `<<<` is a here-string and opens no body.
test("Here-Document rule: a here-document body is data, and the line after it is a command again", () => {
  expect(segmentAndTokenize("a <<EOF --x\nbody\nEOF\nb")).toEqual([["a", "--x"], ["b"]]);
  expect(segmentAndTokenize("a <<'E' <<\"F\" <<\\G\n1\nE\n2\nF\n3\nG\nb")).toEqual([["a"], ["b"]]);
  expect(segmentAndTokenize("a <<-EOF\n\tbody\n\tEOF\nb")).toEqual([["a"], ["b"]]);
  expect(segmentAndTokenize("a <<EOF\nfoo\\\nEOF\nEOF\nb")).toEqual([["a"], ["b"]]);
  expect(segmentAndTokenize("a <<'EOF'\nfoo\\\nEOF\nb")).toEqual([["a"], ["b"]]);
  expect(segmentAndTokenize("a <<EOF\nEOF \nEOF\nb")).toEqual([["a"], ["b"]]);
  expect(segmentAndTokenize('a <<EOF "x\ny"\nbody\nEOF\nb')).toEqual([["a", "x\ny"], ["b"]]);
  expect(segmentAndTokenize('a <<<"x" b\nc')).toEqual([["a", "<<<", "x", "b"], ["c"]]);
  // Innocent: the body names a bypass but is only data.
  expect(classify("git commit -F - <<EOF\ngit commit --no-verify\nEOF", armed).deny).toBe(false);
  expect(classify("git commit -F - <<'EOF'\ngit commit --no-verify\nEOF", armed).deny).toBe(false);
  expect(classify("cat <<-EOF\n\tgit commit --no-verify\n\tEOF\ngit commit -m x", armed).deny).toBe(false);
  expect(classify("cat <<EOF\ngit commit --no-verify \\\nEOF\nEOF\ngit commit -m x", armed).deny).toBe(false);
  expect(classify('grep x <<<"y"\ngit commit -m x', armed).deny).toBe(false);
  // Denied: a flag on the command line beside the operator, or a command after the body ends.
  expect(classify("git commit -F - <<EOF --no-verify\nx\nEOF", armed).deny).toBe(true);
  expect(classify("cat <<EOF\nx\nEOF\ngit commit --no-verify -m y", armed).deny).toBe(true);
  expect(classify("cat <<'EOF'\nx \\\nEOF\ngit commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify('git commit -F - <<<"x" --no-verify', armed).deny).toBe(true);
  expect(classify('grep x <<<"y"\ngit commit --no-verify -m x', armed).deny).toBe(true);
});

// Here-Document inside Command Substitution — the shape a multi-line commit message takes: a here-document inside a
// `$(…)` inside double quotes, whose body may contain apostrophes and double quotes freely.
test("a here-document inside a quoted $(…) keeps the outer span intact whatever its body contains", () => {
  const message = "git commit -m \"$(cat <<'EOF'\nfix: don't \"break\" the gate\n\nthe body names git commit --no-verify\nEOF\n)\"";
  expect(segmentAndTokenize(message)).toEqual([
    ["git", "commit", "-m", "$(cat <<'EOF'\nfix: don't \"break\" the gate\n\nthe body names git commit --no-verify\nEOF\n)"],
  ]);
  expect(classify(message, armed).deny).toBe(false);
  expect(classify(message + " --no-verify", armed).deny).toBe(true);
  expect(classify(message + " && git push --no-verify", armed).deny).toBe(true);
  expect(classify(message + " && git push", armed).deny).toBe(false);
});

test("an unterminated quote or substitution does not throw, and a flag visible before it is still caught", () => {
  // A genuinely unterminated span is malformed input a real shell also refuses to execute (syntax
  // error), so this guard's decision on content trapped inside it cannot itself enable a bypass;
  // what matters is that the tokenizer does not crash, and does not lose a flag that was never
  // inside the broken span to begin with.
  for (const tail of ['"unterminated', "'unterminated", "`unterminated", "$(unterminated", "${unterminated", '"$(a "b', "<<"]) {
    expect(() => classify(`git commit --no-verify -m ${tail}`, armed)).not.toThrow();
    expect(classify(`git commit --no-verify -m ${tail}`, armed).deny).toBe(true);
  }
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
  // A leading assignment before the `export` word, or a `--` ending its options, changes nothing.
  expect(classify("FOO=1 export GIT_CONFIG_COUNT=1; git commit -m x", armed).deny).toBe(true);
  expect(classify("export -- GIT_CONFIG_COUNT=1; git commit -m x", armed).deny).toBe(true);
  expect(classify("export FOO=1 GIT_CONFIG_COUNT=1 && git commit -m x", armed).deny).toBe(true);
});

test("export of an unrelated variable does not deny, and a bare non-exported prefix still does not propagate", () => {
  expect(classify("export FOO=1 && git commit -m x", armed).deny).toBe(false);
  expect(classify('GIT_CONFIG_COUNT=1 echo hi; git commit -m "x"', armed).deny).toBe(false);
});

// `export` is a command, so it counts only at a segment's command position (index 0, or after
// the leading `NAME=value` assignments). The same word as another command's ARGUMENT never
// touches the environment.
test("export is recognised only at the command position of its segment", () => {
  expect(classify('printf export GIT_CONFIG_COUNT=1 && git commit -m "fix bug"', armed).deny).toBe(false);
  expect(classify("echo export GIT_CONFIG_KEY_0=core.hooksPath; git commit -m x", armed).deny).toBe(false);
  expect(classify('git commit -m "export GIT_CONFIG_COUNT=1"', armed).deny).toBe(false);
  expect(classify("git commit -m x; export GIT_CONFIG_COUNT=1", armed).deny).toBe(false);
  // `export -n` removes the export attribute; git never sees the variable.
  expect(classify("export -n GIT_CONFIG_COUNT; git commit -m x", armed).deny).toBe(false);
});
