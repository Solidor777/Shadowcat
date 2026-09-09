import { test, expect } from "vitest";
import {
  parseWorkflowRunSteps,
  parseGateManifest,
  diffManifest,
  unrunnableLocalEntries,
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
  const steps = parseWorkflowRunSteps(wf);
  expect(steps.some((s) => s.job === "push")).toBe(false);
  expect(steps).toContainEqual({ job: "rust", command: "pnpm build", line: 8, multiline: false });
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

test("a '#' inside a manifest command's own quoted value is not read as a trailing comment", () => {
  const toml = `[[gate]]
job = "rust"
command = "echo \\"x\\" # not a comment"
tier = "push"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { job: "rust", command: 'echo "x" # not a comment', tier: "push", reason: "", line: 1 },
  ]);
});

test("an escaped quote inside a manifest command round-trips to a literal quote", () => {
  const toml = `[[gate]]
job = "web"
command = "pnpm --filter \\"pkg\\" build"
tier = "push"
`;
  expect(parseGateManifest(toml, "t.toml")).toEqual([
    { job: "web", command: 'pnpm --filter "pkg" build', tier: "push", reason: "", line: 1 },
  ]);
});

test("a workflow run: line quoting a GitHub Actions expression matches its escaped manifest entry", () => {
  const wf = `jobs:
  rust:
    steps:
      - run: bash scripts/package.sh "\${{ runner.os == 'macOS' && 'macos' || 'linux' }}"
`;
  const toml = `[[gate]]
job = "rust"
command = "bash scripts/package.sh \\"\${{ runner.os == 'macOS' && 'macos' || 'linux' }}\\""
tier = "ci-only"
reason = "Packages per runner OS; one desktop cannot produce every leg."
`;
  const steps = parseWorkflowRunSteps(wf);
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
    { job: "rust", command: 'echo "${{ runner.os }}"', tier: "push", reason: "", line: 1 },
    { job: "rust", command: "cargo fmt --all -- --check", tier: "commit", reason: "", line: 2 },
  ];
  expect(unrunnableLocalEntries([], entries)).toEqual([entries[0]]);
});

test("a ci-only entry with an Actions expression is exempt: it never claims to run locally", () => {
  const entries = [
    { job: "rust", command: 'bash scripts/package.sh "${{ runner.os }}"', tier: "ci-only", reason: "x", line: 1 },
  ];
  expect(unrunnableLocalEntries([], entries)).toEqual([]);
});

test("a single-line commit/push entry referencing a runner-only env var is unrunnable locally, with no ${{ present", () => {
  const entries = [
    { job: "docs", command: 'echo "size" >> "$GITHUB_STEP_SUMMARY"', tier: "push", reason: "", line: 1 },
  ];
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
    const entries = [{ job: "x", command: `echo "$${name}"`, tier: "commit", reason: "", line: 1 }];
    expect(unrunnableLocalEntries([], entries)).toEqual([entries[0]]);
  }
});

test("a commit/push entry sourced from a multi-line `run: |` block is unrunnable locally", () => {
  const steps = [{ job: "rust", command: "a=1 b=2", line: 5, multiline: true }];
  const entries = [{ job: "rust", command: "a=1 b=2", tier: "push", reason: "", line: 1 }];
  expect(unrunnableLocalEntries(steps, entries)).toEqual([entries[0]]);
});

test("an inline (non-block) commit/push entry with no expression is runnable", () => {
  const steps = [{ job: "rust", command: "cargo fmt --all -- --check", line: 5, multiline: false }];
  const entries = [{ job: "rust", command: "cargo fmt --all -- --check", tier: "commit", reason: "", line: 1 }];
  expect(unrunnableLocalEntries(steps, entries)).toEqual([]);
});
