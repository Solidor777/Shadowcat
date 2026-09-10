import { test, expect } from "vitest";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  parseWorkflowRunSteps,
  parseGateManifest,
  diffManifest,
  unrunnableLocalEntries,
  listWorkflowFiles,
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

const STEP = (workflow, job, command, extra = {}) => ({
  workflow,
  job,
  command,
  line: 1,
  multiline: false,
  env: false,
  ...extra,
});
const ENTRY = (workflow, job, command, tier, extra = {}) => ({
  workflow,
  job,
  command,
  tier,
  reason: "",
  line: 1,
  ...extra,
});

test("every run step is found, with its workflow and job", () => {
  expect(parseWorkflowRunSteps(WF, "ci.yml").map((s) => [s.workflow, s.job, s.command])).toEqual([
    ["ci.yml", "rust", "pnpm install --frozen-lockfile"],
    ["ci.yml", "rust", "cargo fmt --all -- --check"],
    ["ci.yml", "rust", 'n=$(nproc 2>/dev/null || echo 2) echo "CARGO_BUILD_JOBS=$n" >> "$GITHUB_ENV"'],
    ["ci.yml", "ui-e2e", "pnpm --filter @shadowcat/shell e2e"],
  ]);
});

test("a parsed `run: |` block carries multiline: true and an inline step carries multiline: false", () => {
  const steps = parseWorkflowRunSteps(WF, "ci.yml");
  const block = steps.find((s) => s.command.startsWith("n=$(nproc"));
  expect(block).toBeDefined();
  expect(block.multiline).toBe(true);
  expect(block.line).toBe(13);
  const inline = steps.find((s) => s.command === "cargo fmt --all -- --check");
  expect(inline.multiline).toBe(false);
});

test("a workflow fixture's `run: |` block missing from the manifest lands in unclassified, keyed on its normalised body", () => {
  const steps = parseWorkflowRunSteps(WF, "ci.yml");
  // Every step in the fixture EXCEPT the block is classified.
  const toml = `[[gate]]
workflow = "ci.yml"
job = "rust"
command = "pnpm install --frozen-lockfile"
tier = "setup"
reason = "installs"

[[gate]]
workflow = "ci.yml"
job = "rust"
command = "cargo fmt --all -- --check"
tier = "commit"

[[gate]]
workflow = "ci.yml"
job = "ui-e2e"
command = "pnpm --filter @shadowcat/shell e2e"
tier = "ci-only"
reason = "browser suite"
`;
  const d = diffManifest(steps, parseGateManifest(toml, "t.toml"));
  expect(d.unclassified).toHaveLength(1);
  expect(d.unclassified[0]).toMatchObject({
    workflow: "ci.yml",
    job: "rust",
    multiline: true,
    command: 'n=$(nproc 2>/dev/null || echo 2) echo "CARGO_BUILD_JOBS=$n" >> "$GITHUB_ENV"',
  });
  expect(d.stale).toEqual([]);
});

test("a 2-space-indented key outside `jobs:` is never treated as a job", () => {
  const wf = `name: CI
on:
  push:
    run: echo not-a-job
jobs:
  rust:
    steps:
      - run: pnpm build
`;
  const steps = parseWorkflowRunSteps(wf, "ci.yml");
  expect(steps.some((s) => s.job === "push")).toBe(false);
  expect(steps).toContainEqual({
    workflow: "ci.yml",
    job: "rust",
    command: "pnpm build",
    line: 8,
    multiline: false,
    env: false,
  });
});

test("a multi-line block body is normalised to one line", () => {
  expect(normCommand("  a=1\n\n  b=2  \n")).toBe("a=1 b=2");
});

// A step's identity is its command PLUS its environment: the same `run:` string under an `env:`
// block is a different invocation, and a manifest entry that stores only the string describes a
// step the local runner cannot reproduce.
test("a step's own `env:` block, written before or after its `run:`, marks the step env: true", () => {
  const wf = `jobs:
  docs:
    steps:
      - name: Before
        env:
          RUSTDOCFLAGS: "-D rustdoc::missing_doc_code_examples"
        run: cargo +nightly doc
      - name: After
        run: cargo doc
        env:
          RUSTDOCFLAGS: "-D warnings"
      - name: Inline mapping
        env: { A: "1" }
        run: pnpm x
      - name: Plain
        run: pnpm lint
`;
  expect(parseWorkflowRunSteps(wf, "ci.yml").map((s) => [s.command, s.env])).toEqual([
    ["cargo +nightly doc", true],
    ["cargo doc", true],
    ["pnpm x", true],
    ["pnpm lint", false],
  ]);
});

test("a job-level `env:` marks every step in that job and none in the next job", () => {
  const wf = `jobs:
  docs:
    env:
      RUSTDOCFLAGS: "-D warnings"
    steps:
      - run: pnpm a
      - run: pnpm b
  web:
    steps:
      - run: pnpm c
`;
  expect(parseWorkflowRunSteps(wf, "ci.yml").map((s) => [s.command, s.env])).toEqual([
    ["pnpm a", true],
    ["pnpm b", true],
    ["pnpm c", false],
  ]);
});

test("a workflow-level `env:` marks every step in every job", () => {
  const wf = `name: CI
env:
  CARGO_TERM_COLOR: always
jobs:
  docs:
    steps:
      - run: pnpm a
  web:
    steps:
      - run: pnpm b
`;
  expect(parseWorkflowRunSteps(wf, "ci.yml").map((s) => [s.command, s.env])).toEqual([
    ["pnpm a", true],
    ["pnpm b", true],
  ]);
});

test("an `env:` key nested under a step's `with:` is not the step's environment", () => {
  const wf = `jobs:
  docs:
    steps:
      - uses: some/action@v1
        with:
          env: production
      - run: pnpm a
`;
  expect(parseWorkflowRunSteps(wf, "ci.yml").map((s) => [s.command, s.env])).toEqual([["pnpm a", false]]);
});

test("a commit/push entry whose workflow step carries an `env:` block is unrunnable locally, end to end through the parser", () => {
  const wf = `jobs:
  docs:
    steps:
      - name: Doc examples present (Rust)
        env:
          RUSTDOCFLAGS: "-D rustdoc::missing_doc_code_examples"
        run: cargo +nightly doc --no-deps
`;
  const toml = `[[gate]]
workflow = "ci.yml"
job = "docs"
command = "cargo +nightly doc --no-deps"
tier = "push"
`;
  const steps = parseWorkflowRunSteps(wf, "ci.yml");
  const entries = parseGateManifest(toml, "t.toml");
  // None of the other three unrunnable shapes is present, so only the env block can flag it.
  expect(steps[0].command).not.toMatch(/\$\{\{|GITHUB_|RUNNER_/);
  expect(steps[0].multiline).toBe(false);
  const d = diffManifest(steps, entries);
  expect(d.unclassified).toEqual([]);
  expect(d.stale).toEqual([]);
  expect(d.unrunnableLocal).toEqual([entries[0]]);
});

test("a ci-only or setup entry whose step carries `env:` is not flagged: neither claims to run locally", () => {
  const steps = [
    STEP("ci.yml", "rust", "bash scripts/package.sh", { env: true }),
    STEP("ci.yml", "rust", "pnpm install", { env: true }),
  ];
  const entries = [
    ENTRY("ci.yml", "rust", "bash scripts/package.sh", "ci-only", { reason: "x" }),
    ENTRY("ci.yml", "rust", "pnpm install", "setup", { reason: "y" }),
  ];
  expect(unrunnableLocalEntries(steps, entries)).toEqual([]);
});

test("the manifest parses entries and rejects an unknown tier", () => {
  const toml = `[[gate]]
workflow = "ci.yml"
job = "rust"
command = "cargo fmt --all -- --check"
tier = "commit"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { workflow: "ci.yml", job: "rust", command: "cargo fmt --all -- --check", tier: "commit", reason: "", line: 1 },
  ]);
  expect(() =>
    parseGateManifest(`[[gate]]\nworkflow = "ci.yml"\njob = "x"\ncommand = "y"\ntier = "later"\n`, "t.toml"),
  ).toThrow(/tier/);
});

test("a manifest entry naming no workflow file is rejected at parse time", () => {
  expect(() => parseGateManifest(`[[gate]]\njob = "x"\ncommand = "y"\ntier = "commit"\n`, "t.toml")).toThrow(
    /workflow/,
  );
});

test("a workflow step absent from the manifest is unclassified", () => {
  const steps = [STEP("ci.yml", "docs", "pnpm lint:comments", { line: 9 })];
  const d = diffManifest(steps, []);
  expect(d.unclassified).toEqual(steps);
  expect(d.stale).toEqual([]);
});

test("a manifest entry absent from the workflow is stale", () => {
  const entries = [ENTRY("ci.yml", "docs", "pnpm lint:gone", "commit")];
  expect(diffManifest([], entries).stale).toEqual([entries[0]]);
});

test("a ci-only entry without a reason is a violation", () => {
  const steps = [STEP("ci.yml", "ui-e2e", "pnpm e2e")];
  const entries = [ENTRY("ci.yml", "ui-e2e", "pnpm e2e", "ci-only")];
  expect(diffManifest(steps, entries).missingReason).toEqual([entries[0]]);
});

// "Not a gate" is a claim like "cannot run locally" is: re-tiering `pnpm lint:comments` to
// `setup` silently stops it running locally, and an unreasoned tier is the one that gets chosen
// under pressure.
test("a setup entry without a reason is a violation, exactly as for ci-only", () => {
  const steps = [STEP("ci.yml", "docs", "pnpm lint:comments")];
  const entries = [ENTRY("ci.yml", "docs", "pnpm lint:comments", "setup")];
  expect(diffManifest(steps, entries).missingReason).toEqual([entries[0]]);
  const reasoned = [ENTRY("ci.yml", "docs", "pnpm lint:comments", "setup", { reason: "installs" })];
  expect(diffManifest(steps, reasoned).missingReason).toEqual([]);
});

test("a commit or push entry needs no reason", () => {
  const steps = [STEP("ci.yml", "docs", "pnpm lint:docs"), STEP("ci.yml", "docs", "pnpm build")];
  const entries = [ENTRY("ci.yml", "docs", "pnpm lint:docs", "commit"), ENTRY("ci.yml", "docs", "pnpm build", "push")];
  expect(diffManifest(steps, entries).missingReason).toEqual([]);
});

test("the same command in two jobs is two independent entries", () => {
  const steps = [STEP("ci.yml", "rust", "pnpm build", { line: 2 }), STEP("ci.yml", "web", "pnpm build", { line: 20 })];
  const entries = [ENTRY("ci.yml", "rust", "pnpm build", "push")];
  expect(diffManifest(steps, entries).unclassified).toEqual([steps[1]]);
});

test("the same job and command in two workflow files is two independent entries", () => {
  const steps = [STEP("ci.yml", "web", "pnpm lint"), STEP("nightly.yml", "web", "pnpm lint")];
  const entries = [ENTRY("ci.yml", "web", "pnpm lint", "commit")];
  expect(diffManifest(steps, entries).unclassified).toEqual([steps[1]]);
  expect(diffManifest(steps, entries).stale).toEqual([]);
});

test("listWorkflowFiles enumerates every .yml and .yaml in the directory, sorted, and nothing else", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-wf-"));
  writeFileSync(join(dir, "nightly.yaml"), "jobs:\n");
  writeFileSync(join(dir, "ci.yml"), "jobs:\n");
  writeFileSync(join(dir, "README.md"), "");
  expect(listWorkflowFiles(dir)).toEqual(["ci.yml", "nightly.yaml"]);
});

test("a '#' inside a manifest command's own quoted value is not read as a trailing comment", () => {
  const toml = `[[gate]]
workflow = "ci.yml"
job = "rust"
command = "echo \\"x\\" # not a comment"
tier = "push"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { workflow: "ci.yml", job: "rust", command: 'echo "x" # not a comment', tier: "push", reason: "", line: 1 },
  ]);
});

test("an escaped quote inside a manifest command round-trips to a literal quote", () => {
  const toml = `[[gate]]
workflow = "ci.yml"
job = "web"
command = "pnpm --filter \\"pkg\\" build"
tier = "push"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { workflow: "ci.yml", job: "web", command: 'pnpm --filter "pkg" build', tier: "push", reason: "", line: 1 },
  ]);
});

test("a workflow run: line quoting a GitHub Actions expression matches its escaped manifest entry", () => {
  const wf = `jobs:
  rust:
    steps:
      - run: bash scripts/package.sh "\${{ runner.os == 'macOS' && 'macos' || 'linux' }}"
`;
  const toml = `[[gate]]
workflow = "ci.yml"
job = "rust"
command = "bash scripts/package.sh \\"\${{ runner.os == 'macOS' && 'macos' || 'linux' }}\\""
tier = "ci-only"
reason = "Packages per runner OS; one desktop cannot produce every leg."
`;
  const steps = parseWorkflowRunSteps(wf, "ci.yml");
  const entries = parseGateManifest(toml, "t.toml");
  expect(diffManifest(steps, entries)).toEqual({
    unclassified: [],
    stale: [],
    missingReason: [],
    unrunnableLocal: [],
  });
});

test("a commit/push entry with an unresolved Actions expression is unrunnable locally", () => {
  const entries = [
    ENTRY("ci.yml", "rust", 'echo "${{ runner.os }}"', "push"),
    ENTRY("ci.yml", "rust", "cargo fmt --all -- --check", "commit"),
  ];
  expect(unrunnableLocalEntries([], entries)).toEqual([entries[0]]);
});

test("a ci-only entry with an Actions expression is exempt: it never claims to run locally", () => {
  const entries = [ENTRY("ci.yml", "rust", 'bash scripts/package.sh "${{ runner.os }}"', "ci-only", { reason: "x" })];
  expect(unrunnableLocalEntries([], entries)).toEqual([]);
});

test("a single-line commit/push entry referencing a runner-only env var is unrunnable locally, with no ${{ present", () => {
  const entries = [ENTRY("ci.yml", "docs", 'echo "size" >> "$GITHUB_STEP_SUMMARY"', "push")];
  expect(entries[0].command).not.toContain("${{");
  expect(unrunnableLocalEntries([], entries)).toEqual([entries[0]]);
});

test("every named runner-only env var is caught, plus the wider GITHUB_*/RUNNER_* family", () => {
  const names = [
    "GITHUB_ENV",
    "GITHUB_OUTPUT",
    "GITHUB_STEP_SUMMARY",
    "GITHUB_WORKSPACE",
    "RUNNER_OS",
    "RUNNER_TEMP",
    "GITHUB_SHA",
    "RUNNER_ARCH",
  ];
  for (const name of names) {
    const entries = [ENTRY("ci.yml", "x", `echo "$${name}"`, "commit")];
    expect(unrunnableLocalEntries([], entries)).toEqual([entries[0]]);
  }
});

test("a commit/push entry sourced from a multi-line `run: |` block is unrunnable locally", () => {
  const steps = [STEP("ci.yml", "rust", "a=1 b=2", { line: 5, multiline: true })];
  const entries = [ENTRY("ci.yml", "rust", "a=1 b=2", "push")];
  expect(unrunnableLocalEntries(steps, entries)).toEqual([entries[0]]);
});

test("an inline (non-block, no-env) commit/push entry with no expression is runnable", () => {
  const steps = [STEP("ci.yml", "rust", "cargo fmt --all -- --check", { line: 5 })];
  const entries = [ENTRY("ci.yml", "rust", "cargo fmt --all -- --check", "commit")];
  expect(unrunnableLocalEntries(steps, entries)).toEqual([]);
});
