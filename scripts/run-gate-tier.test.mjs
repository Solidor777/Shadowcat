import { test, expect, vi } from "vitest";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  symlinkSync,
  statSync,
  utimesSync,
  lutimesSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  tierCommands,
  manifestHash,
  receiptPath,
  writeReceipt,
  readReceipt,
  receiptMatches,
  pushDirtyTreeRefusal,
  pushDirtyTreeRefusalAfterRun,
  headMovedRefusal,
  derivedMtimeExemptions,
  fileTableChangedRefusal,
  captureFileTable,
  parseHeadAndTree,
  buildReceipt,
} from "./run-gate-tier.mjs";
import { parseGateManifest, MANIFEST } from "./check-gate-manifest.mjs";

const RUN_GATE_TIER_SOURCE = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "run-gate-tier.mjs"),
  "utf8",
);

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

test("a setup-tier entry never runs in the push tier, distinctly from ci-only", () => {
  const entries = [E("rust", "pnpm install --frozen-lockfile", "setup")];
  expect(tierCommands(entries, "push")).toEqual([]);
});

test("in the real manifest, `pnpm build` precedes every cargo-prefixed push command", () => {
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const commands = tierCommands(entries, "push");
  const buildIndex = commands.indexOf("pnpm build");
  expect(buildIndex).toBeGreaterThanOrEqual(0);
  const cargoIndexes = commands
    .map((c, i) => (c.startsWith("cargo") ? i : -1))
    .filter((i) => i >= 0);
  expect(cargoIndexes.length).toBeGreaterThan(0);
  for (const i of cargoIndexes) expect(i).toBeGreaterThan(buildIndex);
});

test("a receipt round-trips and matches only its own tree and manifest", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
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

test("a corrupt receipt is reported distinctly from a missing one, not thrown", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  writeFileSync(receiptPath(dir), "{ this is not json");
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(readReceipt(dir)).toBe(null);
  expect(spy).toHaveBeenCalledTimes(1);
  expect(spy.mock.calls[0][0]).toMatch(/corrupt/);
  spy.mockRestore();
});

test("push mode refuses on a dirty tree, before any gate runs", () => {
  expect(pushDirtyTreeRefusal(" M scripts/run-gate-tier.mjs\n")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing.*dirty/),
  });
});

test("push mode proceeds on a clean tree", () => {
  expect(pushDirtyTreeRefusal("")).toEqual({ ok: true, why: "" });
  expect(pushDirtyTreeRefusal("   \n")).toEqual({ ok: true, why: "" });
});

test("the receipt write is refused if the tree went dirty during the run", () => {
  expect(pushDirtyTreeRefusalAfterRun(" M scripts/run-gate-tier.mjs\n")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing to write the receipt.*dirty/),
  });
  expect(pushDirtyTreeRefusalAfterRun("")).toEqual({ ok: true, why: "" });
});

test("the receipt write is refused if HEAD moved during the run", () => {
  expect(headMovedRefusal("aaa111", "bbb222")).toEqual({
    ok: false,
    why: expect.stringMatching(/refusing to write the receipt.*HEAD moved/),
  });
  expect(headMovedRefusal("aaa111", "aaa111")).toEqual({ ok: true, why: "" });
});

test("parseHeadAndTree splits `git rev-parse HEAD HEAD^{tree}`'s two-line stdout", () => {
  expect(parseHeadAndTree("aaa111\nbbb222\n")).toEqual({ sha: "aaa111", tree: "bbb222" });
  expect(parseHeadAndTree("aaa111\nbbb222")).toEqual({ sha: "aaa111", tree: "bbb222" });
});

test("buildReceipt uses the captured sample's sha and tree, never an independent lookup", () => {
  const receipt = buildReceipt(
    { sha: "capturedSha", tree: "capturedTree" },
    "manifestHash123",
    "2020-01-01T00:00:00.000Z",
  );
  expect(receipt).toEqual({
    tree: "capturedTree",
    sha: "capturedSha",
    manifest: "manifestHash123",
    finishedAt: "2020-01-01T00:00:00.000Z",
  });
});

test("buildReceipt's output fields equal its input sample's fields, unmixed and unswapped", () => {
  // This only confirms buildReceipt doesn't drop or swap the fields it's handed — it CANNOT see
  // whether the caller's `{ sha, tree }` sample came from one atomic `git` call or was bundled
  // from two independent calls before being passed in, so it is not, by itself, a regression test
  // for a future call site reintroducing a second `git` call. That call-site regression is what
  // the "only one atomic git call feeds buildReceipt" test below checks instead, by scanning the
  // real source between the GIT-CALL-BUDGET markers.
  const sample = { sha: "s1", tree: "t1" };
  const first = buildReceipt(sample, "m", "t");
  expect(first.sha).toBe(sample.sha);
  expect(first.tree).toBe(sample.tree);
  const otherSample = { sha: "s2", tree: "t2" };
  const second = buildReceipt(otherSample, "m", "t");
  expect(second.sha).toBe(otherSample.sha);
  expect(second.tree).toBe(otherSample.tree);
});

test("only one atomic git call feeds buildReceipt: the post-run block calls `gitOrAbort(` exactly once", () => {
  // Source-scanning regression test for what buildReceipt's own unit tests cannot see (its input's
  // provenance): a future edit that adds a second, independent git-invoking call anywhere between
  // the post-run status check and `writeReceipt` — reintroducing a split-sample receipt — fails
  // this test even though buildReceipt's signature and behavior are untouched. Matches
  // `gitOrAbort(`, the one call every git invocation in this file's direct-entry block routes
  // through (itself a thin wrapper over the shared `runGit` in `scripts/lib/run-git.mjs`).
  //
  // This regex alone has a second door: it does not see a future `runGit(...)` call added
  // directly inside this region, bypassing `gitOrAbort` entirely and reintroducing the same
  // split-sample defect through a route this pattern cannot match. The companion test below
  // closes that door structurally, by asserting `runGit` is referenced nowhere in this file
  // except inside `gitOrAbort`'s own definition — a stronger, file-wide invariant that makes a
  // region-scoped widening of this regex unnecessary.
  const start = RUN_GATE_TIER_SOURCE.indexOf("// GIT-CALL-BUDGET-START");
  const end = RUN_GATE_TIER_SOURCE.indexOf("// GIT-CALL-BUDGET-END");
  expect(start).toBeGreaterThan(-1);
  expect(end).toBeGreaterThan(start);
  const region = RUN_GATE_TIER_SOURCE.slice(start, end);
  const gitCalls = region.match(/\bgitOrAbort\(/g) ?? [];
  expect(gitCalls).toHaveLength(1);
});

test("runGit is referenced exactly once in this file: inside gitOrAbort's own definition", () => {
  // Closes the second door the test above cannot see (see its comment): every OTHER git
  // invocation in this file must go through `gitOrAbort`, never call the shared `runGit`
  // directly, or it evades both this file's abort-wording contract and the GIT-CALL-BUDGET
  // region's one-call invariant. `\brunGit\(` matches a call, not the `import { runGit }` line
  // (no `(` immediately follows `runGit` there), so this counts call sites only.
  const calls = RUN_GATE_TIER_SOURCE.match(/\brunGit\(/g) ?? [];
  expect(calls).toHaveLength(1);
});

test("derivedMtimeExemptions reads the path out of the manifest's `git diff --exit-code <path>` entry", () => {
  const entries = [
    E("rust", "cargo test --all", "push"),
    E("rust", "git diff --exit-code src/types/generated", "push"),
  ];
  expect(derivedMtimeExemptions(entries)).toEqual(["src/types/generated"]);
});

test("derivedMtimeExemptions yields no exemption when no such entry exists — fails toward stricter", () => {
  const entries = [E("rust", "cargo test --all", "push"), E("web", "pnpm lint", "commit")];
  expect(derivedMtimeExemptions(entries)).toEqual([]);
});

test("in the real manifest, the derived mtime exemption matches the bindings-sync gate", () => {
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  expect(derivedMtimeExemptions(entries)).toEqual(["src/types/generated"]);
});

test("fileTableChangedRefusal passes when no tracked file's size or mtime moved", () => {
  const before = { "a.txt": { size: 10n, mtimeNs: 100n }, "b.txt": { size: 20n, mtimeNs: 200n } };
  const after = { "a.txt": { size: 10n, mtimeNs: 100n }, "b.txt": { size: 20n, mtimeNs: 200n } };
  expect(fileTableChangedRefusal(before, after)).toEqual({ ok: true, why: "" });
});

test("fileTableChangedRefusal catches an edit-and-revert: content matches, mtime moved anyway", () => {
  const before = { "a.txt": { size: 10n, mtimeNs: 100n } };
  const after = { "a.txt": { size: 10n, mtimeNs: 999n } };
  const result = fileTableChangedRefusal(before, after);
  expect(result.ok).toBe(false);
  expect(result.why).toMatch(/a\.txt/);
  expect(result.why).toMatch(/reverted mid-run/);
});

// A deterministic cause (a gate step or test that writes to a tracked path itself) is
// indistinguishable, from the file table alone, from a genuine concurrent edit — but the two
// calls for opposite advice: repeating helps one and not the other. The message must name both
// rather than blanket-advising a repeat, which loops an agent for the run's full duration on a
// cause repeating can never fix.
test("fileTableChangedRefusal's advice names both a concurrent edit and a self-writing gate step, not just 'repeat'", () => {
  const before = { "a.txt": { size: 10n, mtimeNs: 100n } };
  const after = { "a.txt": { size: 10n, mtimeNs: 999n } };
  const result = fileTableChangedRefusal(before, after);
  expect(result.why).toMatch(/concurrent edit/);
  expect(result.why).toMatch(/gate step or test writes/);
  expect(result.why).not.toMatch(/^gate:.*Repeat `pnpm gate:push`\.$/);
});

test("fileTableChangedRefusal exempts only the derived prefix, not lookalike paths", () => {
  const before = {
    "src/types/generated/foo.ts": { size: 1n, mtimeNs: 1n },
    "src/types/generated-extra/bar.ts": { size: 1n, mtimeNs: 1n },
  };
  const after = {
    "src/types/generated/foo.ts": { size: 1n, mtimeNs: 999n },
    "src/types/generated-extra/bar.ts": { size: 1n, mtimeNs: 999n },
  };
  const result = fileTableChangedRefusal(before, after, ["src/types/generated"]);
  expect(result.ok).toBe(false);
  expect(result.why).toMatch(/src\/types\/generated-extra\/bar\.ts/);
  expect(result.why).not.toMatch(/generated\/foo\.ts/);
});

test("fileTableChangedRefusal flags a tracked file created or deleted between the two samples", () => {
  const before = { "a.txt": { size: 10n, mtimeNs: 100n } };
  const after = { "a.txt": { size: 10n, mtimeNs: 100n }, "b.txt": { size: 5n, mtimeNs: 5n } };
  expect(fileTableChangedRefusal(before, after).ok).toBe(false);
});

test("fileTableChangedRefusal counts a path that could not be stat'd as CHANGED, in either or both samples", () => {
  const readable = { size: 1n, mtimeNs: 1n };
  // Unreadable at both samples is the case that would otherwise sit silently outside the check.
  const both = fileTableChangedRefusal({ "sub/module": null }, { "sub/module": null });
  expect(both.ok).toBe(false);
  expect(both.why).toMatch(/sub\/module/);
  expect(both.why).toMatch(/could not be read/);
  expect(fileTableChangedRefusal({ "a.txt": null }, { "a.txt": readable }).ok).toBe(false);
  expect(fileTableChangedRefusal({ "a.txt": readable }, { "a.txt": null }).ok).toBe(false);
});

test("captureFileTable records null, not absence, for a path that cannot be stat'd", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  const missing = join(dir, "does-not-exist");
  const table = captureFileTable([missing]);
  expect(Object.keys(table)).toEqual([missing]);
  expect(table[missing]).toBe(null);
});

test("captureFileTable carries bigint mtimeNs and sees a sub-millisecond mtime move", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  const f = join(dir, "a.txt");
  writeFileSync(f, "same content at both samples");
  const base = Math.floor(Date.now() / 1000);
  utimesSync(f, base, base);
  const before = captureFileTable([f]);
  expect(typeof before[f].mtimeNs).toBe("bigint");
  expect(typeof before[f].size).toBe("bigint");
  // One microsecond later: representable in the filesystem's own units on NTFS (100 ns), APFS and
  // ext4 (1 ns), and below the 1 ms a millisecond comparison would fold together.
  utimesSync(f, base, base + 1e-6);
  const after = captureFileTable([f]);
  expect(after[f].mtimeNs - before[f].mtimeNs).toBeGreaterThan(0n);
  expect(after[f].mtimeNs - before[f].mtimeNs).toBeLessThan(1_000_000n);
  expect(fileTableChangedRefusal(before, after).ok).toBe(false);
});

test("captureFileTable observes a link's OWN metadata, not its target's", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  const target = join(dir, "target");
  mkdirSync(target);
  writeFileSync(join(target, "f.txt"), "x");
  const link = join(dir, "link");
  // A directory link is creatable without elevated privileges on every platform: Windows makes a
  // junction, POSIX ignores the type hint and makes an ordinary symlink.
  symlinkSync(target, link, "junction");
  const first = captureFileTable([link]);
  // Move the TARGET's mtime far away from now.
  utimesSync(target, 1000, 1000);
  // Positive control: a following stat does see the target move, so an unchanged table entry
  // below is evidence of lstat, not of nothing having happened.
  expect(statSync(link, { bigint: true }).mtimeNs).toBe(1000n * 1_000_000_000n);
  const targetMoved = captureFileTable([link]);
  expect(fileTableChangedRefusal(first, targetMoved)).toEqual({ ok: true, why: "" });
  // Move the LINK's own mtime, which is what recreating a symlink (edit-and-revert) does.
  lutimesSync(link, 2000, 2000);
  const linkMoved = captureFileTable([link]);
  const result = fileTableChangedRefusal(first, linkMoved);
  expect(result.ok).toBe(false);
  expect(result.why).toContain(link);
});
