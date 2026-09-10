import { test, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { execPath } from "node:process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import {
  classify,
  segmentAndTokenize,
  looksLikeCommitOrPush,
  bestEffortCommand,
  OPTION_TABLES,
} from "../.claude/hooks/guard-git.mjs";

const armed = { hooksPathSet: true };
const bare = { hooksPathSet: false };
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HOOK_PATH = resolve(REPO_ROOT, ".claude", "hooks", "guard-git.mjs");
const SETTINGS_PATH = resolve(REPO_ROOT, ".claude", "settings.json");

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
// Registration. The guard reads `tool_input.command` and nothing else about the tool, so it
// serves every tool that executes a command string (Bash, PowerShell, Monitor, any added later)
// only if the matcher registers it for all of them: a name-based matcher that misses one tool
// loses every denial this layer uniquely provides through that tool. The settings entry is pinned
// here so narrowing it back to one tool fails a test rather than a session.
// ---------------------------------------------------------------------------------------------
test("the guard is registered for every tool, and the entry point gates on the payload's command field alone", () => {
  const settings = JSON.parse(readFileSync(SETTINGS_PATH, "utf8"));
  const entries = settings.hooks.PreToolUse.filter((entry) =>
    entry.hooks.some((h) => h.command.includes("guard-git.mjs")),
  );
  expect(entries).toHaveLength(1);
  expect(entries[0].matcher).toBe("*");

  for (const toolName of ["PowerShell", "Bash", "Monitor"]) {
    const denied = spawnSync(execPath, [HOOK_PATH], {
      input: JSON.stringify({ tool_name: toolName, tool_input: { command: "git commit --no-verify -m x" } }),
      encoding: "utf8",
    });
    expect(denied.stdout, toolName).toContain('"permissionDecision":"deny"');
  }
  // A payload with no command string is a no-op even when another field names a bypass: an edit
  // that writes these very tests must not be refused.
  const edit = spawnSync(execPath, [HOOK_PATH], {
    input: JSON.stringify({
      tool_name: "Edit",
      tool_input: { file_path: "x.mjs", new_string: "git commit --no-verify -m x" },
    }),
    encoding: "utf8",
  });
  expect(edit.stdout.trim()).toBe("");
});

// ---------------------------------------------------------------------------------------------
// git's option grammar. Each case below is derived from a documented rule of git's front end
// (git(1) OPTIONS), the parse-options API (short-option bundling, attached and separate values,
// long-option abbreviation, negation, `--`), or git-config(1) (name folding, modes) — measured
// against git before being pinned here — and every rule that can deny is paired with the innocent
// twin that must stay allowed.
// ---------------------------------------------------------------------------------------------

// parse-options: a short option taking a value takes the REST OF ITS BUNDLE when there is one,
// and the next word only when the bundle ends there. `-mwip` is `-m wip`, so the flag after it is
// a real flag; `-mcleanup` and `-mnit` are messages whose letters happen to include `n`; `-amn` is
// `-a -m n`; `-nm x` and `-anm x` carry a real `-n` before the message option consumes the rest.
test("parse-options bundling: a value-taking short option consumes the rest of its bundle, in both directions", () => {
  expect(classify("git commit -mwip --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -nm x", armed).deny).toBe(true);
  expect(classify("git commit -anm x", armed).deny).toBe(true);
  expect(classify("git commit -m x -an", armed).deny).toBe(true);
  expect(classify("git commit -Fmsg.txt -n", armed).deny).toBe(true);
  expect(classify("git commit -mcleanup", armed).deny).toBe(false);
  expect(classify("git commit -mnit", armed).deny).toBe(false);
  expect(classify("git commit -amn", armed).deny).toBe(false);
  expect(classify("git commit -m nit", armed).deny).toBe(false);
  expect(classify("git commit -Cn", armed).deny).toBe(false);
  expect(classify("git commit -cn", armed).deny).toBe(false);
  expect(classify("git commit -tn -m x", armed).deny).toBe(false);
});

// parse-options: an option with an OPTIONAL value takes only the rest of its bundle (or `=`),
// never the next word — so the word after `-S`, `-u`, `--gpg-sign`, `--force-with-lease` is a
// flag; `-Sn`/`-un` attach `n` as the value.
test("parse-options optional values: attached only, so the next word stays a flag", () => {
  expect(classify("git commit -S --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit -u --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit --gpg-sign --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git push --force-with-lease --no-verify origin main", armed).deny).toBe(true);
  expect(classify("git commit -Sn -m x", armed).deny).toBe(false);
  expect(classify("git commit -un -m x", armed).deny).toBe(false);
  expect(classify("git commit -u=n -m x", armed).deny).toBe(false);
  expect(classify("git commit --gpg-sign=n -m x", armed).deny).toBe(false);
});

// parse-options: an option with a REQUIRED value takes the next word whatever it looks like, so
// `--message --no-verify` is a message reading `--no-verify`, and `--trailer`, `--fixup`,
// `--repo`, `--recurse-submodules` swallow the word after them.
test("parse-options required values: the next word is the value even when it is spelled like a flag", () => {
  expect(classify("git commit --message --no-verify", armed).deny).toBe(false);
  expect(classify("git commit --message=--no-verify", armed).deny).toBe(false);
  expect(classify("git commit --trailer --no-verify -m x", armed).deny).toBe(false);
  expect(classify("git commit --fixup --no-verify", armed).deny).toBe(false);
  expect(classify("git commit -F --no-verify", armed).deny).toBe(false);
  expect(classify("git push --repo --no-verify origin main", armed).deny).toBe(false);
  expect(classify("git push --recurse-submodules --no-verify origin main", armed).deny).toBe(false);
  expect(classify("git push -o --no-verify origin main", armed).deny).toBe(false);
  // The word AFTER the value is a flag again.
  expect(classify("git commit --trailer k=v --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit --author a --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git push --repo origin --no-verify main", armed).deny).toBe(true);
});

// parse-options: a long option may be abbreviated to any prefix naming exactly one option;
// `--no-veri` and `--no-verif` are `--no-verify`. `--no-ver` and `--no-v` are ambiguous between
// `--no-verify` and the negated `--verbose`, which git rejects, so denying them refuses nothing
// that would have run. `--mess x` is `--message x`, whose value must still be dropped.
test("parse-options abbreviation: every unambiguous prefix of --no-verify is denied on commit and push", () => {
  for (const spelling of ["--no-veri", "--no-verif", "--no-verify"]) {
    expect(classify(`git commit ${spelling} -m x`, armed).deny, spelling).toBe(true);
    expect(classify(`git push ${spelling} origin main`, armed).deny, spelling).toBe(true);
  }
  for (const spelling of ["--no-ver", "--no-v"]) {
    expect(classify(`git commit ${spelling} -m x`, armed).deny, spelling).toBe(true);
    expect(classify(`git push ${spelling} origin main`, armed).deny, spelling).toBe(true);
  }
  expect(classify("git commit --mess x --no-verify", armed).deny).toBe(true);
  expect(classify("git commit --m x --no-verify", armed).deny).toBe(true);
  // Innocent twins: an abbreviated message option whose value is spelled like the flag, an
  // abbreviation of an unrelated option, and a prefix of a negatable option that is not verify.
  expect(classify("git commit --mess --no-verify", armed).deny).toBe(false);
  expect(classify("git commit --mess=--no-verify", armed).deny).toBe(false);
  expect(classify("git commit --am -m x", armed).deny).toBe(false);
  expect(classify("git commit --no-stat -m x", armed).deny).toBe(false);
  expect(classify("git push --no-t origin main", armed).deny).toBe(false);
});

// parse-options negation: an option named `no-x` is negated by `--x` and by `--no-no-x`; neither
// bypasses. A `--no-` spelling of a value-taking option takes no value, so the word after it is
// read again.
test("parse-options negation: --verify and --no-no-verify are the opposite of --no-verify, and a negated option takes no value", () => {
  expect(classify("git commit --verify -m x", armed).deny).toBe(false);
  expect(classify("git commit --no-no-verify -m x", armed).deny).toBe(false);
  expect(classify("git push --verify origin main", armed).deny).toBe(false);
  expect(classify("git commit --no-message --no-verify", armed).deny).toBe(true);
  expect(classify("git commit --no-trailer -m x", armed).deny).toBe(false);
});

// parse-options: `--` ends option parsing; everything after it is an operand (a pathspec, a
// refspec), never a flag.
test("parse-options --: a bypass spelling after -- is an operand, before it is a flag", () => {
  expect(classify("git commit -m x -- --no-verify", armed).deny).toBe(false);
  expect(classify("git commit -m x -- -n", armed).deny).toBe(false);
  expect(classify("git push origin main -- --no-verify", armed).deny).toBe(false);
  expect(classify("git commit -m x --no-verify --", armed).deny).toBe(true);
  expect(classify("git commit --no-verify -- x", armed).deny).toBe(true);
  // A pathspec before the flag does not end option parsing for commit or push.
  expect(classify("git commit file.txt --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git push origin main --no-verify", armed).deny).toBe(true);
});

// An option the table does not know is read as a flag taking no value: the word after it stays
// visible, which can only refuse more, never less.
test("an option outside the table is a flag: it hides nothing after it", () => {
  expect(classify("git commit --frobnicate --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git commit --frobnicate -m x", armed).deny).toBe(false);
  expect(classify("git commit -X -m x", armed).deny).toBe(false);
});

// git(1) OPTIONS: the front end's own value-taking options take the NEXT word (`-c`, `-C`,
// `--config-env`, `--git-dir`, `--work-tree`, `--namespace`, `--attr-source`) or, for the long
// ones, `--name=value`; no bundling and no abbreviation apply. A `-c` with its value attached is
// an unknown option to git.
test("git(1) front-end options: separate and attached value forms, no abbreviation", () => {
  expect(classify("git --attr-source HEAD commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git --attr-source=HEAD commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git --namespace ns --work-tree . commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git -p -P --no-pager --bare commit --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git --attr-source HEAD commit -m x", armed).deny).toBe(false);
  expect(classify("git --namespace ns --work-tree . commit -m x", armed).deny).toBe(false);
  expect(classify("git -ccore.hooksPath=/dev/null commit -m x", armed).deny).toBe(false);
  expect(classify("git -c", armed).deny).toBe(false);
  expect(classify("git --config-env", armed).deny).toBe(false);
});

// git-config(1) name folding: section and variable names are case-insensitive, subsection names
// are not. Every spelling of a gate key is the gate key; a key that merely contains one, or has
// a subsection, is not.
test("config name folding: every case spelling of a gate key is the key, in every route", () => {
  for (const key of ["core.hookspath", "CORE.HooksPath", "Core.HOOKSPATH", "core.hooksPath"]) {
    expect(classify(`git -c ${key}=/dev/null commit -m x`, armed).deny, key).toBe(true);
    expect(classify(`git config ${key} /x`, armed).deny, key).toBe(true);
    expect(classify(`git config ${key}`, armed).deny, key).toBe(false);
    expect(classify(`git config --get ${key}`, armed).deny, key).toBe(false);
  }
  for (const key of ["extensions.worktreeconfig", "Extensions.WorktreeConfig"]) {
    expect(classify(`git config ${key} false`, armed).deny, key).toBe(true);
    expect(classify(`git -c ${key}=false commit -m x`, armed).deny, key).toBe(true);
  }
  expect(classify("git -c core.hookspathx=1 commit -m x", armed).deny).toBe(false);
  expect(classify("git -c core.hooks.path=1 commit -m x", armed).deny).toBe(false);
  expect(classify("git -c remote.Core.hookspath=1 commit -m x", armed).deny).toBe(false);
  expect(classify("git config core.hookspathx /x", armed).deny).toBe(false);
});

// git(1) `-c`: omitting the `=` sets the boolean true, `name=` sets the empty string; both write
// the key, and the empty string in particular disables the hooks directory.
test("inline -c: the no-value and empty-value forms of a gate key are writes", () => {
  expect(classify("git -c core.hooksPath commit -m x", armed).deny).toBe(true);
  expect(classify("git -c core.hooksPath= commit -m x", armed).deny).toBe(true);
  expect(classify('git -c "core.hooksPath=" commit -m x', armed).deny).toBe(true);
  expect(classify("git -c core.hooksPath=/x status", armed).deny).toBe(true); // any subcommand
  expect(classify("git -c user.name=x commit -m x", armed).deny).toBe(false);
  expect(classify("git -c commit.gpgsign commit -m x", armed).deny).toBe(false);
});

// git(1) `--config-env=<name>=<envvar>`: the value comes from the environment, so only the name
// is visible — and the name is what selects the gate key.
test("inline --config-env: both the attached and the separate form targeting a gate key are denied", () => {
  expect(classify("H=/dev/null git --config-env=core.hooksPath=H commit -m x", armed).deny).toBe(true);
  expect(classify("H=/dev/null git --config-env core.hooksPath=H commit -m x", armed).deny).toBe(true);
  expect(classify("git --config-env=CORE.HOOKSPATH=H commit -m x", armed).deny).toBe(true);
  expect(classify("git --config-env=extensions.worktreeConfig=H push origin main", armed).deny).toBe(true);
  expect(classify("N=me git --config-env=user.name=N commit -m x", armed).deny).toBe(false);
  expect(classify("git --config-env user.email=E commit -m x", armed).deny).toBe(false);
});

// git(1) ENVIRONMENT: the configuration-carrying variables are `GIT_CONFIG_COUNT`,
// `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>` and `GIT_CONFIG_PARAMETERS`. Their names are matched
// case-insensitively because environment variable names fold case on Windows (a lowercase
// spelling redirects the gate there and is inert elsewhere). `GIT_CONFIG_GLOBAL`/`_SYSTEM`/
// `_NOSYSTEM` select files at scopes the worktree-scoped gate outranks, and are not refused.
test("GIT_CONFIG_* environment: every configuration-carrying name in any case is denied; the file-selecting names are not", () => {
  expect(classify("git_config_count=1 git_config_key_0=core.hookspath git_config_value_0=/dev/null git commit -m x", armed).deny).toBe(true);
  expect(classify("Git_Config_Count=1 git push origin main", armed).deny).toBe(true);
  expect(classify("GIT_CONFIG_PARAMETERS=\"'core.hookspath'='/dev/null'\" git commit -m x", armed).deny).toBe(true);
  expect(classify("export git_config_parameters=\"'core.hookspath'='/dev/null'\"; git commit -m x", armed).deny).toBe(true);
  expect(classify("export GIT_CONFIG_PARAMETERS && git push origin main", armed).deny).toBe(true);
  expect(classify("GIT_CONFIG_GLOBAL=/tmp/cfg git commit -m x", armed).deny).toBe(false);
  expect(classify("GIT_CONFIG_SYSTEM=/tmp/cfg git commit -m x", armed).deny).toBe(false);
  expect(classify("GIT_CONFIG_NOSYSTEM=1 git commit -m x", armed).deny).toBe(false);
  expect(classify("GIT_CONFIGURATION=1 git commit -m x", armed).deny).toBe(false);
  expect(classify("export GIT_CONFIG_GLOBAL=/tmp/cfg; git commit -m x", armed).deny).toBe(false);
});

// git-config(1) Includes: `include.path` and `includeIf.<condition>.path` pull a file in at the
// scope of the directive — at command-line precedence when given inline, above every file scope
// — so the file can carry the gate key. Refused on any subcommand, in any case spelling.
test("inline include directives are denied on any subcommand; keys that merely resemble them are not", () => {
  expect(classify("git -c include.path=/tmp/x commit -m x", armed).deny).toBe(true);
  expect(classify("git -c Include.Path=/tmp/x commit -m x", armed).deny).toBe(true);
  expect(classify("git -c includeIf.gitdir:/tmp/.path=/tmp/x commit -m x", armed).deny).toBe(true);
  expect(classify("git -c 'includeIf.onbranch:Main.path=/x' push origin main", armed).deny).toBe(true);
  expect(classify("git -c include.path=/tmp/x status", armed).deny).toBe(true);
  expect(classify("git --config-env=include.path=P commit -m x", armed).deny).toBe(true);
  expect(classify("GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include.path GIT_CONFIG_VALUE_0=/x git status", armed).deny).toBe(true);
  expect(classify("git -c include.pathx=1 commit -m x", armed).deny).toBe(false);
  expect(classify("git -c includeif.x=1 commit -m x", armed).deny).toBe(false);
  expect(classify("git -c core.include.path=1 commit -m x", armed).deny).toBe(false);
  expect(classify('git commit -m "set include.path in docs"', armed).deny).toBe(false);
});

// git-config(1) `alias.<name>`: an alias defined inline for the subcommand being invoked is
// classified through its expansion — its words are substituted for the subcommand under shell
// splitting, a `!` value runs as a shell command with the remaining arguments appended, and an
// alias cannot shadow `commit`/`push`/`config`. An alias whose expansion this guard cannot read
// (its value lives in the environment) is refused outright.
test("inline aliases are classified through their expansion, in every route and both directions", () => {
  expect(classify("git -c alias.ci='commit --no-verify' ci -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.CI='commit --no-verify' ci -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.ci='commit --no-verify' CI -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.ci=commit ci --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.ci=commit ci -mwip -n", armed).deny).toBe(true);
  expect(classify("git -c alias.a=b -c alias.b='commit --no-verify' a -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.ci='!git commit --no-verify' ci", armed).deny).toBe(true);
  expect(classify("git -c alias.ci='!git commit' ci --no-verify -m x", armed).deny).toBe(true);
  expect(classify("git -c alias.cfg=config cfg core.hookspath /x", armed).deny).toBe(true);
  expect(classify("git -c alias.p='push --no-verify' p origin main", armed).deny).toBe(true);
  expect(classify("GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.ci GIT_CONFIG_VALUE_0='commit --no-verify' git ci -m x", armed).deny).toBe(true);
  expect(classify("GIT_CONFIG_PARAMETERS=\"'alias.ci'='commit --no-verify'\" git ci -m x", armed).deny).toBe(true);
  expect(classify("X='commit --no-verify' git --config-env=alias.ci=X ci -m x", armed).deny).toBe(true);
  expect(classify("git --config-env=alias.ci=X ci -m x", armed).deny).toBe(true); // unreadable
  expect(classify("git -c alias.a=b -c alias.b=a a", armed).deny).toBe(true); // alias loop
  // An alias that expands to a plain commit reaches the armed check like the commit itself.
  expect(classify("git -c alias.ci=commit ci -m x", armed).deny).toBe(false);
  expect(classify("git -c alias.ci=commit ci -m x", bare).deny).toBe(true);
  expect(classify("git -c alias.ci='commit -m' ci wip", armed).deny).toBe(false);
  expect(classify("git -c alias.ci='!git commit' ci -m x", armed).deny).toBe(false);
  expect(classify("git -c alias.l='log --oneline' l -5", armed).deny).toBe(false);
  // An alias defined for a subcommand other than the one invoked changes nothing.
  expect(classify("git -c alias.ci='commit --no-verify' status", armed).deny).toBe(false);
  expect(classify("git -c alias.ci='commit --no-verify' commit -m x", armed).deny).toBe(false);
});

// git-config(1) modes. The subcommand forms: `list`/`get` read; `set`/`unset`/`rename-section`/
// `remove-section`/`edit` write. The deprecated flag forms carry the same split. A section-level
// write to `core` or `extensions` removes the gate key with the section; `edit` writes through an
// editor this guard cannot see.
test("git config subcommand forms: the reads are allowed and every write to a gate key or section is denied", () => {
  expect(classify("git config get core.hooksPath", armed).deny).toBe(false);
  expect(classify("git config get --all core.hooksPath", armed).deny).toBe(false);
  expect(classify("git config get --show-origin extensions.worktreeConfig", armed).deny).toBe(false);
  expect(classify("git config list", armed).deny).toBe(false);
  expect(classify("git config list --show-origin", armed).deny).toBe(false);
  expect(classify("git config set core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config set --worktree core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config set --value=x core.hookspath /y", armed).deny).toBe(true);
  expect(classify("git config unset core.hooksPath", armed).deny).toBe(true);
  expect(classify("git config unset --all extensions.worktreeConfig", armed).deny).toBe(true);
  expect(classify("git config rename-section core junk", armed).deny).toBe(true);
  expect(classify("git config rename-section junk Core", armed).deny).toBe(true);
  expect(classify("git config remove-section core", armed).deny).toBe(true);
  expect(classify("git config remove-section Extensions", armed).deny).toBe(true);
  expect(classify("git config edit", armed).deny).toBe(true);
  expect(classify("git config edit --worktree", armed).deny).toBe(true);
  expect(classify("git config set user.name x", armed).deny).toBe(false);
  expect(classify("git config unset user.name", armed).deny).toBe(false);
  expect(classify("git config remove-section remote.origin", armed).deny).toBe(false);
  expect(classify("git config rename-section remote.origin remote.up", armed).deny).toBe(false);
});

test("git config deprecated flag forms: every write spelling, abbreviated or not, is denied; every read is allowed", () => {
  for (const flag of ["--add", "--ad", "--replace-all", "--rep", "--unset", "--unset-all", "--unset-a"]) {
    expect(classify(`git config ${flag} core.hooksPath /x`, armed).deny, flag).toBe(true);
    expect(classify(`git config ${flag} user.name x`, armed).deny, flag).toBe(false);
  }
  for (const flag of ["--remove-section", "--rem", "--rename-section", "--ren"]) {
    expect(classify(`git config ${flag} core junk`, armed).deny, flag).toBe(true);
    expect(classify(`git config ${flag} extensions junk`, armed).deny, flag).toBe(true);
    expect(classify(`git config ${flag} remote.origin junk`, armed).deny, flag).toBe(false);
  }
  for (const flag of ["--edit", "--ed", "-e", "-le"]) {
    expect(classify(`git config ${flag}`, armed).deny, flag).toBe(true);
  }
  for (const cmd of [
    "git config --get core.hooksPath",
    "git config --get-all core.hooksPath",
    "git config --get-regexp hooks",
    "git config --get-urlmatch core.hooksPath http://x.example",
    "git config --get-color core.hooksPath",
    "git config --get-colorbool core.hooksPath",
    "git config --list",
    "git config -l",
    "git config -lz",
    "git config --type path core.hooksPath",
    "git config --type=path core.hooksPath",
    "git config -t path core.hooksPath",
    "git config --default /d core.hooksPath",
    "git config --file .git/config --get core.hooksPath",
    "git config -f .git/config core.hooksPath",
    "git config --worktree core.hooksPath",
    "git config --show-origin --show-scope core.hooksPath",
  ]) {
    expect(classify(cmd, armed).deny, cmd).toBe(false);
  }
  // With no mode, two or more operands write — wherever the option's own value sits.
  expect(classify("git config core.hooksPath /a pattern", armed).deny).toBe(true);
  expect(classify("git config --comment c core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config --file .git/config core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config -f .git/config core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config --global core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config -- core.hooksPath /x", armed).deny).toBe(true);
  expect(classify("git config --comment c core.hooksPath", armed).deny).toBe(false);
});

// ---------------------------------------------------------------------------------------------
// Drift check: the option tables are the guard's model of git; the installed git's own `-h`
// output is the reference. Every value-taking option git reports must be in the table with the
// same value kind (an option the table reads as a flag that really takes a value can only
// false-deny, but an option the table reads as value-taking that is really a flag would hide the
// word after it — a bypass), and every short letter must agree. Options the installed git does
// not report (an older git) are not checked. Skipped when git is not on PATH.
// ---------------------------------------------------------------------------------------------

/** Parses `git <cmd> -h` into `long -> { short, arg }`, reading `[=<x>]` as optional and ` <x>`/` (x)`/` [x]` as required. */
function parseGitHelp(text) {
  const out = new Map();
  for (const line of text.split("\n")) {
    const m = /^\s{2,}(?:-(\S), )?--(?:\[no-\])?([a-z0-9-]+)(.*)$/.exec(line);
    if (!m) continue;
    const [, short, long, rest] = m;
    let arg = "none";
    if (rest.startsWith("[=")) arg = "optional";
    else if (/^ [<([]/.test(rest)) arg = "required";
    out.set(long, { short, arg });
  }
  return out;
}

/** The merged `-h` output for a subcommand (and, for config, each of its own subcommands), or null when git is unavailable. */
function gitHelp(cmd) {
  const runs = [[cmd]];
  if (cmd === "config") {
    for (const sub of ["list", "get", "set", "unset", "rename-section", "remove-section", "edit"]) runs.push([cmd, sub]);
  }
  const merged = new Map();
  for (const args of runs) {
    const r = spawnSync("git", [...args, "-h"], { cwd: REPO_ROOT, encoding: "utf8" });
    if (r.error) return null;
    for (const [k, v] of parseGitHelp(`${r.stdout}\n${r.stderr}`)) merged.set(k, v);
  }
  return merged;
}

const installedHelp = spawnSync("git", ["--version"], { encoding: "utf8" }).error ? null : true;

test.skipIf(!installedHelp)("the option tables agree with the installed git's own -h output", () => {
  for (const [cmd, table] of Object.entries(OPTION_TABLES)) {
    const help = gitHelp(cmd);
    expect(help, cmd).not.toBeNull();
    expect(help.size, cmd).toBeGreaterThan(5);
    const byLong = new Map(table.specs.map((s) => [s.long, s]));
    for (const [long, { short, arg }] of help) {
      const spec = byLong.get(long);
      if (arg !== "none") {
        expect(spec, `${cmd} --${long} takes a value in git -h but is missing from the table`).toBeDefined();
      }
      if (!spec) continue;
      expect(spec.arg, `${cmd} --${long} value kind`).toBe(arg);
      if (short) expect(spec.short, `${cmd} --${long} short letter`).toBe(short);
    }
    for (const spec of table.specs) {
      if (spec.short && help.has(spec.long)) expect(help.get(spec.long).short, `${cmd} -${spec.short}`).toBe(spec.short);
    }
  }
});

// ---------------------------------------------------------------------------------------------
// The tokenizer implements the POSIX Shell Command Language quoting and token-recognition rules
// (IEEE Std 1003.1-2017 XCU chapter 2: Quoting, Token Recognition, Redirection, Command Substitution, Here-Document) as a grammar. Every case below is derived
// from one rule of that grammar — never from one particular command — and each rule that
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
  // Unquoted substitutions are opaque words too, redirections inside them included.
  expect(segmentAndTokenize("a $(b c) ${d e} `f g` h")).toEqual([["a", "$(b c)", "${d e}", "`f g`", "h"]]);
  expect(segmentAndTokenize("a $(b >c 2>&1) d")).toEqual([["a", "$(b >c 2>&1)", "d"]]);
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

// Redirection rule — an unquoted `<` or `>` ends the word before it; the operator (`<`, `<&`,
// `<>`, `>`, `>>`, `>&`, `>|`, `&>`, `&>>`, and the `<<<` here-string) and its target word are
// removed from the command's arguments wherever they sit, and an unquoted all-digit word directly
// before the operator is its descriptor number, removed with it. The `&`/`|` inside an operator
// is not a chain separator.
test("Redirection rule: the operator, its descriptor number and its target leave the argument list wherever they sit", () => {
  expect(segmentAndTokenize("a 2>&1 b >|f c &>g d <&0 e")).toEqual([["a", "b", "c", "d", "e"]]);
  expect(segmentAndTokenize("a>b c<d e>>f g<>h i &>>j k")).toEqual([["a", "c", "e", "g", "i", "k"]]);
  expect(segmentAndTokenize('a > "o p" b < \'q r\' c')).toEqual([["a", "b", "c"]]);
  expect(segmentAndTokenize("a > 2>&1 b")).toEqual([["a", "b"]]);
  expect(segmentAndTokenize("a2>b c 12>&- d x2>y")).toEqual([["a2", "c", "d", "x2"]]);
  expect(segmentAndTokenize('a <<<"x y" b\nc')).toEqual([["a", "b"], ["c"]]);
  // Denied: the flag glued to an operator, or placed after a redirection, is still a flag; a
  // redirection between an option and its value does not separate them.
  expect(classify("git commit --no-verify>log -m x", armed).deny).toBe(true);
  expect(classify("git commit -m x --no-verify>log", armed).deny).toBe(true);
  expect(classify("git commit -m >log x --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x 2>&1 --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x >|out --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x &>/dev/null --no-verify", armed).deny).toBe(true);
  expect(classify("git commit -m x <&0 --no-verify", armed).deny).toBe(true);
  expect(classify('git commit -F - <<<"x" --no-verify', armed).deny).toBe(true);
  expect(classify("git commit -m x>log -n", armed).deny).toBe(true);
  // Innocent twins: the redirection target is never an argument, the value after a redirection
  // is still the option's value, and a real `&` after a redirection still separates.
  expect(classify("git commit -m >log x", armed).deny).toBe(false);
  expect(classify("git commit -m x >log 2>&1", armed).deny).toBe(false);
  expect(classify("git commit -m x 2>&1 | tail -1", armed).deny).toBe(false);
  expect(classify("git commit -m x > --no-verify", armed).deny).toBe(false);
  expect(classify("git commit -m x 2> -n", armed).deny).toBe(false);
  expect(segmentAndTokenize("git push 2>&1 & git status")).toEqual([["git", "push"], ["git", "status"]]);
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
  expect(segmentAndTokenize('a <<<"x" b\nc')).toEqual([["a", "b"], ["c"]]);
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
  for (const tail of ['"unterminated', "'unterminated", "`unterminated", "$(unterminated", "${unterminated", '"$(a "b', "<<", ">", "2>&"]) {
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
