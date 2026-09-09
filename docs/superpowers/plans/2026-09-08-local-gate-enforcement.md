# Local Gate Enforcement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make it mechanically impossible for an agent to commit or push code that has not passed the local gates CI will run.

**Architecture:** A tracked manifest classifies every `run:` step in `.github/workflows/ci.yml` into a tier; a drift check fails when manifest and workflow diverge, so the gate list can never again be re-derived from prose. A tier runner executes the manifest's `commit` tier (~70s) from a `pre-commit` hook, and the `push` tier (~19m) writes a receipt keyed to the exact tree hash which `pre-push` verifies. A harness-level hook denies every bypass form, including operating in a repository where the hooks are not installed.

**Tech Stack:** Node 25 ESM (`.mjs`), vitest, POSIX shell git hooks, hand-rolled line parsers in the style of `scripts/check-file-lines.mjs`.

**Spec:** `docs/superpowers/specs/2026-09-08-local-gate-enforcement-design.md`

## Global Constraints

- **No new dependencies.** The repo hand-parses its TOML allowlists (`parseAllowlist`) and `js-yaml` is only a transitive dep. Both new parsers are hand-rolled line scanners. Adding a dependency requires the owner's consent and is out of scope for this plan.
- **Cross-platform from day one.** Hooks must run on Windows (git's bundled bash), macOS and Linux. No GNU-only flags, no assumed binary suffixes, no hardcoded path separators. Build paths with `node:path`.
- **No lint suppressions.** `#[allow(...)]`, `#[expect(...)]`, `eslint-disable` of `no-unused-vars`, `@ts-ignore`/`@ts-nocheck` all require the owner's per-instance sign-off. Fix the code instead.
- **File-size limits.** 5,000 lines soft (needs an allowlist entry the owner signs), 10,000 hard. Test lines count.
- **No PII.** No absolute user paths, addresses, or credentials in any tracked file. This plan makes one previously-untracked file tracked; that file was audited clean and a gate keeps it that way.
- **Deletion.** `rm`/`Remove-Item` are banned. Use `trash`.
- **Comments** state present-tense constraints and invariants. No history, no ticket ids, no narration.
- **Every new script gets a `*.test.mjs` sibling** under `scripts/`; `pnpm run test:scripts` is CI-enforced.

## Model/Effort directives

Plan written mainline (owner's choice at the handoff checkpoint) on Opus 5.
Dispatch loop owned mainline in the same session (owner's choice at the execution checkpoint).

Execution roles, per `~/.claude/docs/sdd-model-effort-tiers.md` **and** the project rule in
`.claude/CLAUDE.md` that mandates agent identity independently of tier:

| Role | Agent | Effort |
| --- | --- | --- |
| Per-task implementer | `shadowcat-codebase:shadowcat-coder` | medium |
| Implementer escalation | `shadowcat-codebase:shadowcat-coder-opus` | high |
| Per-task review (pair) | `shadowcat-codebase:shadowcat-code-reviewer` + `shadowcat-codebase:shadowcat-spec-reviewer` | high |
| Review escalation | the `-opus` twin of whichever reviewer read shallow | high |

Memory records an "opus banned for subagents" directive scoped to the 2026-08-19 docs campaign.
It is not assumed to apply here; if the owner reaffirms it, the escalation tier becomes the
`-fable` twins instead.

## Buddy-check directives

Tasks 3 and 4 qualify as high-risk: a defective `pre-commit` hook or bypass guard blocks the
write path for every agent in the repository, and the failure mode is one the agent hitting it
cannot fix without the very tools it just blocked. Both tasks get the two-reviewer pair rather
than a single reviewer, and their review runs **before** the hooks are installed on the owner's
machine.

---

### Task 1: Gate manifest and drift check

**Files:**
- Create: `scripts/gates.toml`
- Create: `scripts/check-gate-manifest.mjs`
- Create: `scripts/check-gate-manifest.test.mjs`
- Modify: `package.json` (add `lint:gate-manifest`)

**Interfaces:**
- Consumes: `norm`, `under` from `scripts/lib/gate-corpus.mjs`; `isDirectEntry` from `scripts/lib/is-main.mjs`.
- Produces:
  - `parseWorkflowRunSteps(yamlText) -> [{ job, command, line }]`
  - `parseGateManifest(tomlText, sourceName) -> [{ job, command, tier, reason, line }]`
  - `diffManifest(steps, entries) -> { unclassified, stale, missingReason }`
  - `normCommand(text) -> string`
  - Exported constants `MANIFEST = "scripts/gates.toml"`, `WORKFLOW = ".github/workflows/ci.yml"`, `TIERS = ["commit", "push", "setup", "ci-only"]`

- [ ] **Step 1: Write the failing test**

Create `scripts/check-gate-manifest.test.mjs`:

```javascript
import { test, expect } from "vitest";
import {
  parseWorkflowRunSteps,
  parseGateManifest,
  diffManifest,
  normCommand,
} from "./check-gate-manifest.mjs";

const WF = `name: CI
on:
  push:
jobs:
  rust:
    steps:
      - run: pnpm install --frozen-lockfile
      - name: Format
        if: runner.os == 'Linux'
        run: cargo fmt --all -- --check
      - name: Build parallelism
        shell: bash
        run: |
          n=$(nproc 2>/dev/null || echo 2)
          echo "CARGO_BUILD_JOBS=$n" >> "$GITHUB_ENV"
  ui-e2e:
    steps:
      - run: pnpm --filter @shadowcat/shell e2e
`;

test("every run step is found, with its job", () => {
  expect(parseWorkflowRunSteps(WF).map((s) => [s.job, s.command])).toEqual([
    ["rust", "pnpm install --frozen-lockfile"],
    ["rust", "cargo fmt --all -- --check"],
    ["rust", 'n=$(nproc 2>/dev/null || echo 2) echo "CARGO_BUILD_JOBS=$n" >> "$GITHUB_ENV"'],
    ["ui-e2e", "pnpm --filter @shadowcat/shell e2e"],
  ]);
});

test("a job name containing digits or dashes is not missed", () => {
  expect(parseWorkflowRunSteps(WF).some((s) => s.job === "ui-e2e")).toBe(true);
});

test("a multi-line block body is normalised to one line", () => {
  expect(normCommand("  a=1\n\n  b=2  \n")).toBe("a=1 b=2");
});

test("the manifest parses entries and rejects an unknown tier", () => {
  const toml = `[[gate]]
job = "rust"
command = "cargo fmt --all -- --check"
tier = "commit"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { job: "rust", command: "cargo fmt --all -- --check", tier: "commit", reason: "", line: 1 },
  ]);
  expect(() => parseGateManifest(`[[gate]]\njob = "x"\ncommand = "y"\ntier = "later"\n`, "t.toml")).toThrow(
    /tier/,
  );
});

test("a workflow step absent from the manifest is unclassified", () => {
  const steps = [{ job: "docs", command: "pnpm lint:comments", line: 9 }];
  const d = diffManifest(steps, []);
  expect(d.unclassified).toEqual([{ job: "docs", command: "pnpm lint:comments", line: 9 }]);
  expect(d.stale).toEqual([]);
});

test("a manifest entry absent from the workflow is stale", () => {
  const entries = [{ job: "docs", command: "pnpm lint:gone", tier: "commit", reason: "", line: 1 }];
  expect(diffManifest([], entries).stale).toEqual([entries[0]]);
});

test("a ci-only entry without a reason is a violation", () => {
  const steps = [{ job: "ui-e2e", command: "pnpm e2e", line: 3 }];
  const entries = [{ job: "ui-e2e", command: "pnpm e2e", tier: "ci-only", reason: "", line: 1 }];
  expect(diffManifest(steps, entries).missingReason).toEqual([entries[0]]);
});

test("the same command in two jobs is two independent entries", () => {
  const steps = [
    { job: "rust", command: "pnpm build", line: 2 },
    { job: "web", command: "pnpm build", line: 20 },
  ];
  const entries = [{ job: "rust", command: "pnpm build", tier: "push", reason: "", line: 1 }];
  expect(diffManifest(steps, entries).unclassified).toEqual([steps[1]]);
});
```

- [ ] **Step 2: Run the test and watch it fail**

Run: `pnpm vitest run scripts/check-gate-manifest.test.mjs`
Expected: FAIL — cannot resolve `./check-gate-manifest.mjs`.

- [ ] **Step 3: Implement the parsers and the diff**

Create `scripts/check-gate-manifest.mjs`. The workflow scanner is line-based, in the style of
`scripts/check-file-lines.mjs`'s `parseAllowlist`, because the repo hand-parses its own config
formats rather than carrying a parser dependency.

```javascript
// Fails when scripts/gates.toml and .github/workflows/ci.yml disagree about which gates exist.
//
// The gate list is otherwise prose: twenty-odd `run:` steps an agent must transcribe by hand,
// and a transcription that can drop an entry eventually drops one. Classifying every step here
// makes the list an artifact, and makes divergence from CI a gate in its own right — a new
// workflow step that nobody classified fails at the next commit rather than at the next push.
//
// INVARIANT: every `run:` step in the workflow has exactly one manifest entry, keyed by
// (job, normalised command). A command repeated across jobs is a distinct entry per job.

import { readFileSync } from "node:fs";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export const MANIFEST = "scripts/gates.toml";
export const WORKFLOW = ".github/workflows/ci.yml";
export const TIERS = ["commit", "push", "setup", "ci-only"];

/** Collapses whitespace runs so a `run: |` block and its manifest entry compare equal. */
export function normCommand(text) {
  return text.replace(/\s+/g, " ").trim();
}

const JOB = /^ {2}([A-Za-z0-9_-]+):\s*$/;
const RUN_INLINE = /^\s*-?\s*run:\s*(\S.*)$/;
const RUN_BLOCK = /^(\s*)-?\s*run:\s*[|>][-+]?\s*$/;

/** Every `run:` step in the workflow, with the job that contains it. */
export function parseWorkflowRunSteps(yamlText) {
  const lines = yamlText.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let job = "";
  for (let i = 0; i < lines.length; i++) {
    const j = lines[i].match(JOB);
    if (j) {
      job = j[1];
      continue;
    }
    const block = lines[i].match(RUN_BLOCK);
    if (block) {
      const indent = block[1].length;
      const body = [];
      let k = i + 1;
      for (; k < lines.length; k++) {
        const bare = lines[k].trim();
        if (bare === "") {
          body.push("");
          continue;
        }
        if (lines[k].search(/\S/) <= indent) break;
        body.push(bare);
      }
      out.push({ job, command: normCommand(body.join(" ")), line: i + 1 });
      i = k - 1;
      continue;
    }
    const inline = lines[i].match(RUN_INLINE);
    if (inline) out.push({ job, command: normCommand(inline[1]), line: i + 1 });
  }
  return out;
}

/** Parses the restricted `[[gate]]` TOML subset the manifest uses. */
export function parseGateManifest(text, sourceName) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let cur = null;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.replace(/#.*$/, "").trim();
    if (line === "") continue;
    if (line === "[[gate]]") {
      cur = { job: "", command: "", tier: "", reason: "", line: i + 1 };
      out.push(cur);
      continue;
    }
    const kv = line.match(/^(job|command|tier|reason)\s*=\s*"(.*)"$/);
    if (!kv || !cur) {
      throw new Error(
        `${sourceName}:${i + 1}: cannot parse. Expected [[gate]] or job/command/tier/reason = "value". got: ${raw}`,
      );
    }
    cur[kv[1]] = kv[1] === "command" ? normCommand(kv[2]) : kv[2];
  }
  for (const e of out) {
    if (!TIERS.includes(e.tier)) {
      throw new Error(
        `${sourceName}:${e.line}: tier must be one of ${TIERS.join(", ")}. got: ${e.tier || "(none)"}`,
      );
    }
  }
  return out;
}

const key = (x) => `${x.job}\u0000${x.command}`;

/** Workflow steps with no entry, entries with no step, and ci-only entries with no reason. */
export function diffManifest(steps, entries) {
  const byEntry = new Set(entries.map(key));
  const byStep = new Set(steps.map(key));
  return {
    unclassified: steps.filter((s) => !byEntry.has(key(s))),
    stale: entries.filter((e) => !byStep.has(key(e))),
    missingReason: entries.filter((e) => e.tier === "ci-only" && e.reason.trim() === ""),
  };
}

if (isDirectEntry(import.meta.url)) {
  const steps = parseWorkflowRunSteps(readFileSync(WORKFLOW, "utf8"));
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const d = diffManifest(steps, entries);
  const bad = d.unclassified.length + d.stale.length + d.missingReason.length;
  for (const s of d.unclassified) {
    console.error(`${WORKFLOW}:${s.line}: [${s.job}] unclassified gate: ${s.command}`);
  }
  for (const e of d.stale) {
    console.error(`${MANIFEST}:${e.line}: [${e.job}] entry names no workflow step: ${e.command}`);
  }
  for (const e of d.missingReason) {
    console.error(`${MANIFEST}:${e.line}: ci-only entry needs a reason: ${e.command}`);
  }
  if (bad > 0) {
    console.error(
      `\nlint:gate-manifest: ${bad} discrepancy(ies). Classify each workflow step in ${MANIFEST}.`,
    );
    process.exit(1);
  }
  console.log(`lint:gate-manifest: ${steps.length} workflow step(s), ${entries.length} entry(ies), 0 error(s)`);
}
```

- [ ] **Step 4: Run the test and watch it pass**

Run: `pnpm vitest run scripts/check-gate-manifest.test.mjs`
Expected: PASS, 8 tests.

- [ ] **Step 5: Write the manifest against the real workflow**

Create `scripts/gates.toml`. Generate the skeleton from the real workflow rather than by hand:

```bash
node -e "import('./scripts/check-gate-manifest.mjs').then(m=>{for(const s of m.parseWorkflowRunSteps(require('fs').readFileSync('.github/workflows/ci.yml','utf8')))console.log('[[gate]]\njob = \"'+s.job+'\"\ncommand = \"'+s.command.replace(/\"/g,'\\\\\"')+'\"\ntier = \"\"\n')})"
```

Then classify every entry. Tiers, decided in the spec and reproduced here in full:

| Job | Command | Tier | Reason (ci-only only) |
| --- | --- | --- | --- |
| rust | `pnpm install --frozen-lockfile` | setup | |
| rust | `pnpm build` | push | |
| rust | build-parallelism export block | setup | |
| rust | `cargo fmt --all -- --check` | commit | |
| rust | `cargo clippy --all-targets -- -D warnings` | push | |
| rust | `cargo test --all` | push | |
| rust | `git diff --exit-code src/types/generated` | push | |
| rust | `cargo build --release` | push | |
| rust | binary-size-budget block | push | |
| rust | `bash scripts/package.sh ...` | ci-only | Packages per runner OS through a `runner.os` expression; one desktop cannot produce the macOS and Linux bundles the matrix legs build. |
| web | `pnpm install --frozen-lockfile` | setup | |
| web | `pnpm -r typecheck` | push | |
| web | `pnpm -r test` | push | |
| web | `pnpm run test:scripts` | commit | |
| web | `pnpm docs:check-examples` | push | |
| web | `pnpm lint` | commit | |
| web | `pnpm build` | push | |
| web | `pnpm --filter "shadowcat-example-*" build` | push | |
| web | `pnpm run check:svelte-runtime` | commit | |
| e2e | `pnpm install --frozen-lockfile` | setup | |
| e2e | `pnpm build` | setup | |
| e2e | `cargo build -p shadowcat --bin test_server` | ci-only | Builds the binary the ci-only cross-runtime spec run consumes; running it locally without that run buys nothing. |
| e2e | `pnpm --filter @shadowcat/core test:e2e` | ci-only | Cross-runtime suite against a spawned server on a fixed port; a parallel local session poisons the run. |
| ui-e2e | `pnpm install --frozen-lockfile` | setup | |
| ui-e2e | `playwright install --with-deps chromium` | setup | |
| ui-e2e | `pnpm --filter @shadowcat/shell e2e` | ci-only | Browser suite on a fixed port, contention-sensitive and intermittently hanging; blocking a push on it would block for reasons unrelated to the diff. |
| docs | `pnpm install --frozen-lockfile` | setup | |
| docs | `pnpm build:all` | push | |
| docs | `pnpm lint:docs` | commit | |
| docs | `pnpm lint:props` | commit | |
| docs | `pnpm lint:comments` | commit | |
| docs | `pnpm lint:allowances` | commit | |
| docs | `pnpm lint:file-size` | commit | |
| docs | `pnpm lint:inline-tests` | commit | |
| docs | `pnpm lint:aria-labels` | commit | |
| docs | `pnpm docs:check-examples` | push | |
| docs | `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items` | push | |
| docs | `cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc` | push | |

Head the file with a comment stating the invariant and that `pnpm lint:gate-manifest` enforces it.

- [ ] **Step 6: Add the script and run the real check**

In `package.json` scripts add:

```json
"lint:gate-manifest": "node scripts/check-gate-manifest.mjs",
```

Run: `pnpm lint:gate-manifest`
Expected: `0 error(s)`. If it reports unclassified steps, the table above is missing a row that the workflow actually contains — add it rather than deleting the step.

- [ ] **Step 7: Prove the check fires on real drift**

Append a genuine new step to `.github/workflows/ci.yml`'s `docs` job:

```yaml
      - name: Sabotage probe
        run: pnpm lint:nonexistent
```

Run: `pnpm lint:gate-manifest`
Expected: FAIL naming `[docs] unclassified gate: pnpm lint:nonexistent`.

Now revert that exact hunk — the edit was two lines appended to one job, so remove those two lines directly. Do **not** `git checkout --` the file: it carries no other uncommitted work only if nothing else touched it, and reverting the hunk is unconditional.

Run: `pnpm lint:gate-manifest` and `git diff --stat .github/workflows/ci.yml`
Expected: `0 error(s)`, and an empty diff.

- [ ] **Step 8: Commit**

```bash
pnpm lint:gate-manifest && pnpm vitest run scripts/check-gate-manifest.test.mjs && \
git add scripts/gates.toml scripts/check-gate-manifest.mjs scripts/check-gate-manifest.test.mjs package.json && \
git commit -m "build(gates): classify every CI step in a drift-checked manifest"
```

---

### Task 2: Tier runner and push receipt

**Files:**
- Create: `scripts/run-gate-tier.mjs`
- Create: `scripts/run-gate-tier.test.mjs`
- Modify: `package.json` (add `gate:commit`, `gate:push`)

**Interfaces:**
- Consumes: `parseGateManifest`, `MANIFEST`, `normCommand` from `scripts/check-gate-manifest.mjs`.
- Produces:
  - `tierCommands(entries, tier) -> [string]`
  - `manifestHash(text) -> string`
  - `receiptPath(gitDir) -> string`
  - `writeReceipt(gitDir, { tree, sha, manifest, finishedAt })`
  - `readReceipt(gitDir) -> object | null`
  - `receiptMatches(receipt, { tree, manifest }) -> { ok: boolean, why: string }`

- [ ] **Step 1: Write the failing test**

Create `scripts/run-gate-tier.test.mjs`:

```javascript
import { test, expect } from "vitest";
import { mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  tierCommands,
  manifestHash,
  writeReceipt,
  readReceipt,
  receiptMatches,
} from "./run-gate-tier.mjs";

const E = (job, command, tier) => ({ job, command, tier, reason: "", line: 1 });

test("the commit tier is exactly the commit entries, in workflow order", () => {
  const entries = [
    E("rust", "cargo fmt --all -- --check", "commit"),
    E("rust", "cargo test --all", "push"),
    E("web", "pnpm lint", "commit"),
    E("ui-e2e", "pnpm e2e", "ci-only"),
    E("docs", "pnpm install --frozen-lockfile", "setup"),
  ];
  expect(tierCommands(entries, "commit")).toEqual(["cargo fmt --all -- --check", "pnpm lint"]);
});

test("the push tier contains the commit tier and preserves workflow order", () => {
  const entries = [
    E("rust", "pnpm build", "push"),
    E("rust", "cargo fmt --all -- --check", "commit"),
    E("rust", "cargo test --all", "push"),
  ];
  expect(tierCommands(entries, "push")).toEqual([
    "pnpm build",
    "cargo fmt --all -- --check",
    "cargo test --all",
  ]);
});

test("a command repeated across jobs runs once, at its first position", () => {
  const entries = [
    E("rust", "pnpm build", "push"),
    E("web", "pnpm build", "push"),
    E("web", "pnpm -r test", "push"),
  ];
  expect(tierCommands(entries, "push")).toEqual(["pnpm build", "pnpm -r test"]);
});

test("setup and ci-only never run locally", () => {
  const entries = [E("e2e", "pnpm --filter @shadowcat/core test:e2e", "ci-only")];
  expect(tierCommands(entries, "push")).toEqual([]);
});

test("a receipt round-trips and matches only its own tree and manifest", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  mkdirSync(join(dir, "sub"), { recursive: true });
  const m = manifestHash('[[gate]]\njob = "x"\n');
  writeReceipt(dir, { tree: "aaa", sha: "sha1", manifest: m, finishedAt: "t" });
  const r = readReceipt(dir);
  expect(r.tree).toBe("aaa");
  expect(receiptMatches(r, { tree: "aaa", manifest: m }).ok).toBe(true);
  expect(receiptMatches(r, { tree: "bbb", manifest: m }).ok).toBe(false);
  expect(receiptMatches(r, { tree: "bbb", manifest: m }).why).toMatch(/tree/);
  expect(receiptMatches(r, { tree: "aaa", manifest: "other" }).why).toMatch(/manifest/);
});

test("a missing receipt is reported, not thrown", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  expect(readReceipt(dir)).toBe(null);
  expect(receiptMatches(null, { tree: "aaa", manifest: "m" })).toEqual({
    ok: false,
    why: "no receipt: run `pnpm gate:push`",
  });
});
```

- [ ] **Step 2: Run the test and watch it fail**

Run: `pnpm vitest run scripts/run-gate-tier.test.mjs`
Expected: FAIL — cannot resolve `./run-gate-tier.mjs`.

- [ ] **Step 3: Implement the runner**

Create `scripts/run-gate-tier.mjs`:

```javascript
// Runs a tier of the gate manifest, and records a receipt the pre-push hook verifies.
//
// The push tier costs ~19 minutes, which cannot run inside `git push`: an agent's shell tool
// caps at 600 seconds, and a killed push is exactly the pressure that produces `--no-verify`.
// So the tier runs explicitly and leaves a receipt; the hook checks the receipt.
//
// The receipt is keyed to a TREE HASH, never a timestamp. Gates run against a working tree, so
// a receipt keyed to time can describe a tree other than the one being pushed — the defect this
// mechanism exists to remove.
//
// Order is workflow order, so the client build precedes the cargo steps that embed dist/.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { parseGateManifest, MANIFEST } from "./check-gate-manifest.mjs";

const INCLUDED = { commit: ["commit"], push: ["commit", "push"] };

/** The commands a tier runs, deduped to first occurrence, in workflow order. */
export function tierCommands(entries, tier) {
  const want = INCLUDED[tier];
  if (!want) throw new Error(`unknown tier: ${tier}`);
  const seen = new Set();
  const out = [];
  for (const e of entries) {
    if (!want.includes(e.tier) || seen.has(e.command)) continue;
    seen.add(e.command);
    out.push(e.command);
  }
  return out;
}

/** Identity of the gate list; a changed manifest invalidates every outstanding receipt. */
export function manifestHash(text) {
  return createHash("sha256").update(text).digest("hex").slice(0, 16);
}

export function receiptPath(gitDir) {
  return join(gitDir, "shadowcat-gate-receipt");
}

export function writeReceipt(gitDir, receipt) {
  writeFileSync(receiptPath(gitDir), JSON.stringify(receipt, null, 2) + "\n");
}

export function readReceipt(gitDir) {
  const p = receiptPath(gitDir);
  if (!existsSync(p)) return null;
  try {
    return JSON.parse(readFileSync(p, "utf8"));
  } catch {
    return null;
  }
}

/** Whether a receipt describes exactly this tree under exactly this gate list. */
export function receiptMatches(receipt, { tree, manifest }) {
  if (!receipt) return { ok: false, why: "no receipt: run `pnpm gate:push`" };
  if (receipt.tree !== tree) {
    return { ok: false, why: `receipt is for tree ${receipt.tree}, not ${tree}: re-run \`pnpm gate:push\`` };
  }
  if (receipt.manifest !== manifest) {
    return { ok: false, why: "the gate manifest changed since the receipt: re-run `pnpm gate:push`" };
  }
  return { ok: true, why: "" };
}

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

if (isDirectEntry(import.meta.url)) {
  const [mode, arg] = process.argv.slice(2);
  const text = readFileSync(MANIFEST, "utf8");
  const entries = parseGateManifest(text, MANIFEST);
  const hash = manifestHash(text);
  const gitDir = git("rev-parse", "--absolute-git-dir");

  if (mode === "--verify-receipt") {
    const r = receiptMatches(readReceipt(gitDir), { tree: arg, manifest: hash });
    if (!r.ok) {
      console.error(`gate: refusing the push — ${r.why}`);
      process.exit(1);
    }
    console.log("gate: receipt matches the tree being pushed");
    process.exit(0);
  }

  const commands = tierCommands(entries, mode);
  console.log(`gate:${mode} — ${commands.length} step(s)`);
  for (const cmd of commands) {
    const started = Date.now();
    process.stdout.write(`  ${cmd} ... `);
    try {
      execFileSync(cmd, { shell: true, stdio: ["ignore", "pipe", "pipe"] });
    } catch (err) {
      console.log("FAIL");
      process.stderr.write(String(err.stdout ?? "") + String(err.stderr ?? ""));
      console.error(`\ngate:${mode} FAILED at: ${cmd}`);
      process.exit(1);
    }
    console.log(`ok ${Math.round((Date.now() - started) / 1000)}s`);
  }

  if (mode === "push") {
    writeReceipt(gitDir, {
      tree: git("rev-parse", "HEAD^{tree}"),
      sha: git("rev-parse", "HEAD"),
      manifest: hash,
      finishedAt: new Date().toISOString(),
    });
    console.log("gate:push green — receipt written");
  }
}
```

- [ ] **Step 4: Run the test and watch it pass**

Run: `pnpm vitest run scripts/run-gate-tier.test.mjs`
Expected: PASS, 6 tests.

- [ ] **Step 5: Add the scripts and run the commit tier for real**

In `package.json` scripts add:

```json
"gate:commit": "node scripts/run-gate-tier.mjs commit",
"gate:push": "node scripts/run-gate-tier.mjs push",
```

Run: `pnpm gate:commit`
Expected: every step `ok`, total under ~90s. If a step fails, that is a real violation in the tree — fix it, do not reclassify the gate.

- [ ] **Step 6: Commit**

```bash
pnpm gate:commit && pnpm vitest run scripts/run-gate-tier.test.mjs && \
git add scripts/run-gate-tier.mjs scripts/run-gate-tier.test.mjs package.json && \
git commit -m "build(gates): run a manifest tier and record a tree-keyed receipt"
```

---

### Task 3: Git hooks and their installer

**Files:**
- Create: `scripts/git-hooks/pre-commit`
- Create: `scripts/git-hooks/pre-push`
- Create: `scripts/install-git-hooks.mjs`
- Create: `scripts/install-git-hooks.test.mjs`
- Modify: `package.json` (add `prepare`)

**Interfaces:**
- Consumes: nothing from earlier tasks at module level; the hooks invoke `node scripts/run-gate-tier.mjs` as a subprocess.
- Produces:
  - `hooksDir(repoRoot) -> string`
  - `shouldSkip(env) -> boolean`
  - `configPlan(repoRoot) -> [{ args: [string] }]`

- [ ] **Step 1: Write the failing test**

Create `scripts/install-git-hooks.test.mjs`:

```javascript
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

test("the plan enables worktree config before writing a worktree-scoped hooksPath", () => {
  const plan = configPlan("/repo");
  expect(plan[0].args).toEqual(["config", "extensions.worktreeConfig", "true"]);
  expect(plan[1].args[0]).toBe("config");
  expect(plan[1].args[1]).toBe("--worktree");
  expect(plan[1].args[2]).toBe("core.hooksPath");
  expect(plan[1].args[3].replace(/\\/g, "/")).toBe("/repo/scripts/git-hooks");
});
```

- [ ] **Step 2: Run the test and watch it fail**

Run: `pnpm vitest run scripts/install-git-hooks.test.mjs`
Expected: FAIL — cannot resolve `./install-git-hooks.mjs`.

- [ ] **Step 3: Write the two hooks**

Create `scripts/git-hooks/pre-commit`:

```sh
#!/bin/sh
# Blocks a commit that has not passed the manifest's commit tier (~70s).
#
# Runs against the WORKING TREE, not the staged index: several checkers resolve paths relative
# to the repository root and one additionally scans an external skills checkout, so gating an
# exported index would silently change the corpus they scan. The exact gate is the push receipt,
# which is keyed to the tree actually being pushed.
set -e
exec node scripts/run-gate-tier.mjs commit
```

Create `scripts/git-hooks/pre-push`:

```sh
#!/bin/sh
# Blocks a push whose tree has no green receipt from `pnpm gate:push`.
#
# The push tier costs ~19 minutes and cannot run here: an agent's shell tool caps at 600s, and a
# killed push is the pressure that produces --no-verify. Verifying a receipt keeps the push
# instant while keeping the guarantee exact.
#
# INVARIANT: a dirty working tree fails, because the receipt describes HEAD's tree and an unclean
# tree means the gated content is not what the commit contains.
set -e

if [ -n "$(git status --porcelain)" ]; then
    echo "gate: refusing the push — the working tree is dirty. Commit or stash, then re-run \`pnpm gate:push\`." >&2
    exit 1
fi

while read -r _local_ref local_sha _remote_ref _remote_sha; do
    # An all-zero local sha is a branch deletion; there is no tree to gate.
    case "$local_sha" in
        *[!0]*) ;;
        *) continue ;;
    esac
    tree=$(git rev-parse "$local_sha^{tree}")
    node scripts/run-gate-tier.mjs --verify-receipt "$tree" || exit 1
done

exit 0
```

- [ ] **Step 4: Write the installer**

Create `scripts/install-git-hooks.mjs`:

```javascript
// Points this worktree's core.hooksPath at the tracked hooks, so a fresh clone arms itself.
//
// Wired to `prepare`, which pnpm runs on install — the one command every clone and every new
// worktree already runs, so arming needs nobody to remember it.
//
// Scoped per worktree: sibling worktrees share one .git/config, so a repository-wide hooksPath
// would make every worktree run one checkout's hooks. extensions.worktreeConfig moves the
// setting into the per-worktree config, so each runs its own branch's.

import { chmodSync, existsSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export function hooksDir(repoRoot) {
  return join(repoRoot, "scripts", "git-hooks");
}

/** CI checks out fresh per job and pushes nothing; arming it would only slow the runner. */
export function shouldSkip(env) {
  return Boolean(env.CI || env.GITHUB_ACTIONS);
}

/** The git config invocations, in order. worktreeConfig must be enabled before --worktree works. */
export function configPlan(repoRoot) {
  return [
    { args: ["config", "extensions.worktreeConfig", "true"] },
    { args: ["config", "--worktree", "core.hooksPath", hooksDir(repoRoot)] },
  ];
}

if (isDirectEntry(import.meta.url)) {
  if (shouldSkip(process.env)) {
    console.log("install-git-hooks: skipped under CI");
    process.exit(0);
  }
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
  const dir = hooksDir(root);
  if (!existsSync(dir)) {
    console.error(`install-git-hooks: ${dir} is missing`);
    process.exit(1);
  }
  for (const step of configPlan(root)) {
    execFileSync("git", step.args, { stdio: "inherit" });
  }
  // core.filemode is false on Windows checkouts, so the bit is set explicitly rather than
  // relying on the checkout to carry it.
  for (const name of readdirSync(dir)) chmodSync(join(dir, name), 0o755);
  console.log(`install-git-hooks: core.hooksPath -> ${dir}`);
}
```

In `package.json` scripts add:

```json
"prepare": "node scripts/install-git-hooks.mjs",
```

- [ ] **Step 5: Run the test and watch it pass**

Run: `pnpm vitest run scripts/install-git-hooks.test.mjs`
Expected: PASS, 3 tests.

- [ ] **Step 6: Arm the hooks and verify both fire**

Run: `node scripts/install-git-hooks.mjs`
Then: `git config --get --worktree core.hooksPath`
Expected: the absolute path to `scripts/git-hooks`.

Prove `pre-push` refuses without a receipt:

Run: `git push --dry-run origin HEAD`
Expected: FAIL with `no receipt: run \`pnpm gate:push\``.

Prove it refuses a stale receipt. Run `pnpm gate:push` (~19m), then make any commit, then:

Run: `git push --dry-run origin HEAD`
Expected: FAIL naming the tree mismatch, because the new commit has a different tree.

- [ ] **Step 7: Commit**

```bash
pnpm gate:commit && pnpm vitest run scripts/install-git-hooks.test.mjs && \
git add scripts/git-hooks scripts/install-git-hooks.mjs scripts/install-git-hooks.test.mjs package.json && \
git commit -m "build(gates): gate commits on the fast tier and pushes on a receipt"
```

---

### Task 4: Harness bypass guard

**Files:**
- Create: `.claude/hooks/guard-git.mjs`
- Create: `scripts/guard-git.test.mjs`
- Modify: `.claude/settings.json` (add the hook)
- Modify: `.gitignore` (remove the `.claude/settings.json` entry at line 39)

**Interfaces:**
- Consumes: nothing.
- Produces: `classify(command, { hooksPathSet }) -> { deny: boolean, reason: string }`, exported from `.claude/hooks/guard-git.mjs`.

- [ ] **Step 1: Write the failing test**

Create `scripts/guard-git.test.mjs`:

```javascript
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
```

- [ ] **Step 2: Run the test and watch it fail**

Run: `pnpm vitest run scripts/guard-git.test.mjs`
Expected: FAIL — cannot resolve `../.claude/hooks/guard-git.mjs`.

- [ ] **Step 3: Implement the guard**

Create `.claude/hooks/guard-git.mjs`:

```javascript
// Denies every route around the git gates, and denies git writes in a repository with no gates.
//
// The owner's ruling is that no bypass exists: an agent that cannot pass the gate stops and
// reports. The last rule is what makes the git layer non-optional — without it an un-armed
// clone permits everything silently, and "impossible" degrades to "usually".
//
// Flag scanning stops at -m/--message: a commit message legitimately contains things like
// "-n", and treating message text as flags would deny honest commits.

import { execFileSync } from "node:child_process";
import process from "node:process";

const SEGMENT = /[;&|]{1,2}|\n/;

function isGit(tokens) {
  return tokens[0] === "git";
}

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

/** Whether this command must be refused, and what to tell the agent. */
export function classify(command, { hooksPathSet }) {
  for (const segment of String(command).split(SEGMENT)) {
    const tokens = segment.trim().split(/\s+/).filter(Boolean);
    if (!isGit(tokens)) continue;

    if (tokens.some((t) => t.startsWith("core.hooksPath=") || t === "core.hooksPath")) {
      return {
        deny: true,
        reason:
          "Refused: core.hooksPath is the gate. It is not overridable or reconfigurable by an agent.",
      };
    }
    if (tokens.some((t) => t.startsWith("extensions.worktreeConfig"))) {
      return {
        deny: true,
        reason: "Refused: extensions.worktreeConfig carries the per-worktree hooksPath.",
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
          "Refused: --no-verify. There is no bypass. If the gate cannot pass, stop and report it to the owner.",
      };
    }
    if (!hooksPathSet) {
      return {
        deny: true,
        reason:
          "Refused: this repository has no gate installed (core.hooksPath is unset). Run `pnpm install` to arm it.",
      };
    }
  }
  return { deny: false, reason: "" };
}

function hooksPathSet() {
  for (const args of [
    ["config", "--get", "--worktree", "core.hooksPath"],
    ["config", "--get", "core.hooksPath"],
  ]) {
    try {
      if (execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim()) {
        return true;
      }
    } catch {
      // git exits non-zero when the key is unset; try the next scope.
    }
  }
  return false;
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop())) {
  let raw = "";
  process.stdin.on("data", (c) => (raw += c));
  process.stdin.on("end", () => {
    let command = "";
    try {
      command = JSON.parse(raw)?.tool_input?.command ?? "";
    } catch {
      command = "";
    }
    const v = classify(command, { hooksPathSet: hooksPathSet() });
    if (v.deny) {
      process.stdout.write(
        JSON.stringify({
          hookSpecificOutput: {
            hookEventName: "PreToolUse",
            permissionDecision: "deny",
            permissionDecisionReason: v.reason,
          },
        }),
      );
    }
    process.exit(0);
  });
}
```

- [ ] **Step 4: Run the test and watch it pass**

Run: `pnpm vitest run scripts/guard-git.test.mjs`
Expected: PASS, 8 tests.

- [ ] **Step 5: Wire the hook and track the settings file**

In `.claude/settings.json`, add to the existing `hooks.PreToolUse` array:

```json
{
  "matcher": "Bash",
  "hooks": [{ "type": "command", "command": "node \"$CLAUDE_PROJECT_DIR/.claude/hooks/guard-git.mjs\"" }]
}
```

In `.gitignore`, delete the `.claude/settings.json` line. Leave the
`.claude/settings.local.json` line: that file holds machine-specific paths and stays untracked.

- [ ] **Step 6: Verify the deny actually fires end-to-end**

Run: `echo '{"tool_input":{"command":"git commit --no-verify -m x"}}' | node .claude/hooks/guard-git.mjs`
Expected: JSON containing `"permissionDecision":"deny"`.

Run: `echo '{"tool_input":{"command":"git status"}}' | node .claude/hooks/guard-git.mjs`
Expected: empty output.

- [ ] **Step 7: Commit**

```bash
pnpm gate:commit && pnpm vitest run scripts/guard-git.test.mjs && \
git add .claude/hooks/guard-git.mjs scripts/guard-git.test.mjs .claude/settings.json .gitignore && \
git commit -m "build(gates): deny every bypass route at the harness layer"
```

---

### Task 5: Privacy gate, CI wiring, and documentation

**Files:**
- Create: `scripts/check-tracked-settings-privacy.mjs`
- Create: `scripts/check-tracked-settings-privacy.test.mjs`
- Modify: `package.json` (add `lint:settings-privacy`)
- Modify: `.github/workflows/ci.yml` (two steps in the `docs` job)
- Modify: `scripts/gates.toml` (classify those two steps)
- Modify: `.claude/CLAUDE.md`

**Interfaces:**
- Consumes: `isDirectEntry` from `scripts/lib/is-main.mjs`.
- Produces: `scanPrivacy(text) -> [{ line, kind, excerpt }]`, `TRACKED = [".claude/settings.json"]`.

- [ ] **Step 1: Write the failing test**

Create `scripts/check-tracked-settings-privacy.test.mjs`:

```javascript
import { test, expect } from "vitest";
import { scanPrivacy } from "./check-tracked-settings-privacy.mjs";

test("an absolute Windows user path is a violation", () => {
  const f = scanPrivacy('{"a":"C:\\\\Users\\\\someone\\\\notes.txt"}');
  expect(f.map((x) => x.kind)).toEqual(["user path"]);
});

test("absolute unix home paths are violations", () => {
  expect(scanPrivacy('{"a":"/home/someone/x"}')[0].kind).toBe("user path");
  expect(scanPrivacy('{"a":"/Users/someone/x"}')[0].kind).toBe("user path");
});

test("an address is a violation", () => {
  expect(scanPrivacy('{"a":"someone@example.com"}')[0].kind).toBe("address");
});

test("a credential-shaped value is a violation", () => {
  expect(scanPrivacy('{"api_key":"abcdefgh12345678"}')[0].kind).toBe("credential");
  expect(scanPrivacy('{"token": "abcdefgh12345678"}')[0].kind).toBe("credential");
});

test("the portable forms the settings file actually uses are clean", () => {
  expect(scanPrivacy('{"command":"[ -f \\"$CLAUDE_PROJECT_DIR/graphify-out/graph.json\\" ]"}')).toEqual([]);
  expect(scanPrivacy('{"deny":["Bash(rm *)","PowerShell(Remove-Item *)"]}')).toEqual([]);
});

test("the line number of each finding is reported", () => {
  expect(scanPrivacy('{\n"a":1,\n"b":"/home/someone/x"\n}')[0].line).toBe(3);
});
```

- [ ] **Step 2: Run the test and watch it fail**

Run: `pnpm vitest run scripts/check-tracked-settings-privacy.test.mjs`
Expected: FAIL — cannot resolve the module.

- [ ] **Step 3: Implement the privacy check**

Create `scripts/check-tracked-settings-privacy.mjs`:

```javascript
// Fails when a tracked agent-configuration file carries a user path, an address, or a credential.
//
// .claude/settings.json is tracked so the bypass guard survives a fresh clone. Tracking a file
// that is otherwise machine-local puts it one careless edit away from committing personal data,
// and "remember not to" is the discipline this whole mechanism exists to replace.
//
// Machine-specific settings belong in .claude/settings.local.json, which stays untracked.

import { readFileSync, existsSync } from "node:fs";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export const TRACKED = [".claude/settings.json"];

const RULES = [
  { kind: "user path", re: /(?:[A-Za-z]:\\{1,2}Users\\{1,2}|\/home\/|\/Users\/)[A-Za-z0-9._-]+/ },
  { kind: "address", re: /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/ },
  {
    kind: "credential",
    re: /["'](?:api[_-]?key|apikey|token|secret|password|passwd)["']\s*:\s*["'][^"']{8,}["']/i,
  },
];

/** Every privacy violation in `text`, with the line it sits on. */
export function scanPrivacy(text) {
  const out = [];
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  for (let i = 0; i < lines.length; i++) {
    for (const rule of RULES) {
      const m = lines[i].match(rule.re);
      if (m) out.push({ line: i + 1, kind: rule.kind, excerpt: m[0] });
    }
  }
  return out;
}

if (isDirectEntry(import.meta.url)) {
  let bad = 0;
  for (const path of TRACKED) {
    if (!existsSync(path)) continue;
    for (const f of scanPrivacy(readFileSync(path, "utf8"))) {
      // The excerpt is deliberately not printed: echoing the value would copy it into CI logs.
      console.error(`${path}:${f.line}: ${f.kind} in a tracked file`);
      bad++;
    }
  }
  if (bad > 0) {
    console.error(
      `\nlint:settings-privacy: ${bad} violation(s). Move machine-specific settings to .claude/settings.local.json.`,
    );
    process.exit(1);
  }
  console.log(`lint:settings-privacy: ${TRACKED.length} file(s) scanned, 0 error(s)`);
}
```

- [ ] **Step 4: Run the test and watch it pass**

Run: `pnpm vitest run scripts/check-tracked-settings-privacy.test.mjs`
Expected: PASS, 6 tests.

- [ ] **Step 5: Wire both new checks into CI and the manifest**

In `package.json` scripts add:

```json
"lint:settings-privacy": "node scripts/check-tracked-settings-privacy.mjs",
```

In `.github/workflows/ci.yml`, in the `docs` job after the `pnpm lint:aria-labels` step:

```yaml
      - name: Gate manifest matches CI
        run: pnpm lint:gate-manifest
      - name: No personal data in tracked agent settings
        run: pnpm lint:settings-privacy
```

In `scripts/gates.toml` add the two matching entries, both `tier = "commit"`.

Run: `pnpm lint:gate-manifest`
Expected: `0 error(s)` — the manifest now accounts for the two steps it just gained.

- [ ] **Step 6: Record the mechanism in the project instructions**

In `.claude/CLAUDE.md`, under "Autonomous Commits & Milestone Pushes", state that the tiers are
enforced by `core.hooksPath` hooks and a tree-keyed receipt, that `pnpm gate:push` must be run
before a push, and that there is no bypass. Keep it to the existing section's register — a
present-tense constraint, no history.

- [ ] **Step 7: Run the full push tier and commit**

```bash
pnpm gate:push && \
git add scripts/check-tracked-settings-privacy.mjs scripts/check-tracked-settings-privacy.test.mjs \
        scripts/gates.toml package.json .github/workflows/ci.yml .claude/CLAUDE.md && \
git commit -m "build(gates): gate tracked settings for personal data and wire both checks into CI"
```

Expected: `gate:push green — receipt written`, then the commit. The commit changes the tree, so
the receipt is now stale by design — re-run `pnpm gate:push` before pushing.

---

## Completion gates

Beyond the per-task gates, before this branch merges:

- [ ] `pnpm gate:push` green on the final tree, and the push actually blocked when tried against a stale receipt.
- [ ] The `shadowcat-codebase` plugin checkout updated: this changes how every agent commits and pushes, which is subsystem knowledge. Update the affected `shadowcat-codebase-*` skill(s) in `~/.claude/skills/shadowcat-codebase/`, dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff, run `node scripts/check-skill-symbol-refs-cli.mjs` locally, and commit/push inside that repository — it has its own remote and is not part of any commit here.
- [ ] `docs/HISTORY.md` appended; `docs/PLAN.md` and `docs/TODO.md` reconciled against what was actually built.
