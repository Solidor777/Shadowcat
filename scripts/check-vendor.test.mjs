import { test, expect } from "vitest";
import { Buffer } from "node:buffer";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { create } from "tar";
import {
  PATCHED_FILES,
  applyPatch,
  checkUpstream,
  checkVendor,
  functionBody,
  hashFile,
  hunkCounts,
  parseRecord,
  runGate,
} from "./check-vendor.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CLI = resolve(HERE, "check-vendor.mjs");
const REPO = resolve(HERE, "..");

const STATE_UP = `impl State {
    pub fn handle_error(&mut self, err: &proto::Error) {
        match self.inner {
            Closed(..) => {}
            _ => {}
        }
    }

    pub fn recv_eof(&mut self) {
        match self.inner {
            Closed(..) => {}
            _ => {}
        }
    }
}
`;
const STATE_PATCHED = STATE_UP.replace(
  "    pub fn handle_error(&mut self, err: &proto::Error) {\n        match self.inner {\n",
  "    pub fn handle_error(&mut self, err: &proto::Error) {\n        match self.inner {\n            Closed(Cause::ScheduledLibraryReset(_)) => self.inner = Closed(Cause::Error(err.clone())),\n",
).replace(
  "    pub fn recv_eof(&mut self) {\n        match self.inner {\n",
  "    pub fn recv_eof(&mut self) {\n        match self.inner {\n            Closed(Cause::ScheduledLibraryReset(_)) => self.inner = Closed(Cause::Error(broken_pipe())),\n",
);
const SERVER_UP = `impl Peer {
    fn convert_poll_message(pseudo: Pseudo, stream_id: StreamId) {
        let b = Request::builder();
        b.build()
    }
}
`;
const SERVER_PATCHED = SERVER_UP.replace(
  "        let b = Request::builder();\n",
  "        let b = Request::builder();\n        b = b.extension(crate::StreamId::from_internal(stream_id));\n",
);
const LICENSE = "Permission is hereby granted, free of charge, to any person obtaining a copy\n";
const CARGO = '[package]\nname = "h2"\nversion = "0.4.19"\n';

/** One unified-diff section built from a before/after pair that differ by inserted lines only. */
function sectionFor(file, before, after) {
  const a = before.split("\n");
  const b = after.split("\n");
  let out = `--- a/${file}\n+++ b/${file}\n`;
  let i = 0;
  let j = 0;
  while (i < a.length) {
    if (a[i] === b[j]) {
      i++;
      j++;
      continue;
    }
    const inserted = [];
    while (a[i] !== b[j]) inserted.push(b[j++]);
    const ctxBefore = a[i - 1];
    const ctxAfter = a[i];
    out += `@@ -${i},2 +${i},${2 + inserted.length} @@\n ${ctxBefore}\n${inserted.map((l) => `+${l}`).join("\n")}\n ${ctxAfter}\n`;
  }
  return out;
}

const DIFF =
  sectionFor("src/proto/streams/state.rs", STATE_UP, STATE_PATCHED) +
  sectionFor("src/server.rs", SERVER_UP, SERVER_PATCHED);

const ROOT_TOML = `[workspace]\nmembers = ["src/server"]\nexclude = ["vendor/h2"]\n\n[patch.crates-io]\nh2 = { path = "vendor/h2" }\n`;
const LOCK_OK = `[[package]]\nname = "h2"\nversion = "0.4.19"\ndependencies = []\n\n[[package]]\nname = "other"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`;

/** The vendored tree of a passing fixture, keyed by repo-relative path (upstream tree separately). */
function fixture() {
  const upstream = new Map([
    ["Cargo.toml", CARGO],
    ["LICENSE", LICENSE],
    ["src/lib.rs", "pub mod x;\n"],
    ["src/proto/streams/state.rs", STATE_UP],
    ["src/server.rs", SERVER_UP],
    ["data.bin", "a\r\nb"],
  ]);
  const vendored = new Map(upstream);
  vendored.set("src/proto/streams/state.rs", STATE_PATCHED);
  vendored.set("src/server.rs", SERVER_PATCHED);
  vendored.set("SHADOWCAT-PATCH.diff", DIFF);
  vendored.set("SHADOWCAT-PATCH.md", "prose\n");
  const record = [...vendored.keys()]
    .filter((f) => f !== "SHADOWCAT-PATCH.md")
    .sort()
    .map((f) => {
      const v = hashFile(f, Buffer.from(vendored.get(f), "latin1"));
      const u = upstream.has(f) ? hashFile(f, Buffer.from(upstream.get(f), "latin1")) : "-";
      return `${f}  ${u}  ${v}`;
    })
    .join("\n");
  vendored.set(".vendored-sha256", `${record}\n`);
  return { upstream, vendored };
}

/** Inputs for `checkVendor` over a fixture, with optional overrides. */
function inputs(vendored, { root = ROOT_TOML, lock = LOCK_OK, extraPaths = [] } = {}) {
  const files = new Map([...vendored].map(([f, c]) => [`vendor/h2/${f}`, Buffer.from(c, "latin1")]));
  return {
    paths: [...files.keys(), ...extraPaths],
    readFile: (p) => files.get(p) ?? Buffer.from(""),
    rootCargoToml: root,
    cargoLock: lock,
  };
}

test("a sound fixture passes", () => {
  const { vendored } = fixture();
  expect(checkVendor(inputs(vendored))).toEqual([]);
});

test("a stray file under vendor/ outside a named crate fails", () => {
  const { vendored } = fixture();
  const errors = checkVendor(inputs(vendored, { extraPaths: ["vendor/notes.rs"] }));
  expect(errors.some((e) => e.startsWith("vendor/notes.rs: not under a directory"))).toBe(true);
});

test("a stray file inside a vendored crate has no record", () => {
  const { vendored } = fixture();
  vendored.set("src/extra.rs", "fn x() {}\n");
  const errors = checkVendor(inputs(vendored));
  expect(errors.some((e) => e.includes("src/extra.rs: no record"))).toBe(true);
});

test("a modified file outside PATCHED_FILES fails on its hash", () => {
  const { vendored } = fixture();
  vendored.set("src/lib.rs", "pub mod y;\n");
  const errors = checkVendor(inputs(vendored));
  expect(errors.some((e) => e.includes("src/lib.rs: hash"))).toBe(true);
});

test("one changed byte in a patched file fails on its hash", () => {
  const { vendored } = fixture();
  vendored.set("src/server.rs", vendored.get("src/server.rs").replace("Request", "Reqvest"));
  const errors = checkVendor(inputs(vendored));
  expect(errors.some((e) => e.includes("src/server.rs: hash"))).toBe(true);
});

test("a record with a different upstream hash on an unpatched file breaks the PATCHED_FILES equality", () => {
  const { vendored } = fixture();
  const rec = parseRecord(vendored.get(".vendored-sha256")).records;
  const lib = rec.get("src/lib.rs");
  vendored.set("src/lib.rs", "pub mod y;\n");
  const newHash = hashFile("src/lib.rs", Buffer.from("pub mod y;\n"));
  vendored.set(
    ".vendored-sha256",
    vendored.get(".vendored-sha256").replace(`${lib.upstream}  ${lib.vendored}`, `${lib.upstream}  ${newHash}`),
  );
  const errors = checkVendor(inputs(vendored));
  expect(errors.some((e) => e.includes("records that differ from upstream"))).toBe(true);
});

test("a record without a file and a file without a record both fail", () => {
  const { vendored } = fixture();
  vendored.set(".vendored-sha256", `${vendored.get(".vendored-sha256")}gone.rs  ${"0".repeat(64)}  ${"0".repeat(64)}\n`);
  expect(checkVendor(inputs(vendored)).some((e) => e.includes("record for gone.rs has no file"))).toBe(true);
  const fresh = fixture().vendored;
  fresh.set("later.rs", "x\n");
  expect(checkVendor(inputs(fresh)).some((e) => e.includes("later.rs: no record"))).toBe(true);
});

test("the record and the patch prose are exempt from records", () => {
  const { vendored } = fixture();
  expect(vendored.has("SHADOWCAT-PATCH.md")).toBe(true);
  expect(checkVendor(inputs(vendored))).toEqual([]);
});

test("CRLF is folded for text files and left alone for others", () => {
  const lf = Buffer.from("a\nb\n");
  const crlf = Buffer.from("a\r\nb\r\n");
  expect(hashFile("src/x.rs", crlf)).toBe(hashFile("src/x.rs", lf));
  expect(hashFile("LICENSE", crlf)).toBe(hashFile("LICENSE", lf));
  expect(hashFile("data.bin", crlf)).not.toBe(hashFile("data.bin", lf));
});

test("a lock entry with a source fails", () => {
  const { vendored } = fixture();
  const lock = LOCK_OK.replace('version = "0.4.19"\n', 'version = "0.4.19"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n');
  expect(checkVendor(inputs(vendored, { lock })).some((e) => e.includes("has a source"))).toBe(true);
});

test("a lock with two h2 packages fails, with the vendored one alone it passes", () => {
  const { vendored } = fixture();
  const registry = `\n[[package]]\nname = "h2"\nversion = "0.4.18"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`;
  const dup = checkVendor(inputs(vendored, { lock: LOCK_OK + registry }));
  expect(dup.some((e) => e.includes("exactly one h2 package, found 2"))).toBe(true);
  expect(checkVendor(inputs(vendored, { lock: LOCK_OK }))).toEqual([]);
});

test("a lock version that differs from the vendored Cargo.toml fails", () => {
  const { vendored } = fixture();
  const lock = LOCK_OK.replace('version = "0.4.19"', 'version = "0.4.20"');
  expect(checkVendor(inputs(vendored, { lock })).some((e) => e.includes("differs from Cargo.lock"))).toBe(true);
});

test("a missing LICENSE fails", () => {
  const { vendored } = fixture();
  vendored.delete("LICENSE");
  expect(checkVendor(inputs(vendored)).some((e) => e.includes("LICENSE: missing"))).toBe(true);
});

test("an exclude array without the entry fails even when a comment names it", () => {
  const { vendored } = fixture();
  const root = `[workspace]\n# exclude = ["vendor/h2"]\nmembers = ["src/server"]\nexclude = []\n\n[patch.crates-io]\nh2 = { path = "vendor/h2" }\n`;
  expect(checkVendor(inputs(vendored, { root })).some((e) => e.includes("exclude does not list vendor/h2"))).toBe(true);
});

test("a patch table that does not map h2 to the vendored path fails", () => {
  const { vendored } = fixture();
  const root = ROOT_TOML.replace('path = "vendor/h2"', 'path = "elsewhere"');
  expect(checkVendor(inputs(vendored, { root })).some((e) => e.includes("[patch.crates-io] does not map h2"))).toBe(true);
});

test("deleting a hunk from the committed diff fails the hunk count", () => {
  const { vendored } = fixture();
  const diff = vendored.get("SHADOWCAT-PATCH.diff");
  const idx = diff.indexOf("@@", diff.indexOf("@@") + 2);
  const cut = diff.slice(0, idx) + diff.slice(diff.indexOf("--- a/src/server.rs"));
  vendored.set("SHADOWCAT-PATCH.diff", cut);
  const record = parseRecord(vendored.get(".vendored-sha256")).records;
  const old = record.get("SHADOWCAT-PATCH.diff");
  vendored.set(
    ".vendored-sha256",
    vendored.get(".vendored-sha256").replace(old.vendored, hashFile("SHADOWCAT-PATCH.diff", Buffer.from(cut))),
  );
  const errors = checkVendor(inputs(vendored));
  expect(errors.some((e) => e.includes("src/proto/streams/state.rs carries 1 hunk(s), expected 2"))).toBe(true);
});

test("reverting an arm body fails the arm assertion even when its record is updated", () => {
  for (const arm of ["handle_error", "recv_eof"]) {
    const { vendored } = fixture();
    const body = functionBody(STATE_PATCHED, arm);
    const reverted = vendored.get("src/proto/streams/state.rs").replace(body, functionBody(STATE_UP, arm));
    vendored.set("src/proto/streams/state.rs", reverted);
    const old = parseRecord(vendored.get(".vendored-sha256")).records.get("src/proto/streams/state.rs");
    vendored.set(
      ".vendored-sha256",
      vendored.get(".vendored-sha256").replace(old.vendored, hashFile("src/proto/streams/state.rs", Buffer.from(reverted))),
    );
    const errors = checkVendor(inputs(vendored));
    expect(errors.some((e) => e.includes(`State::${arm} has no ScheduledLibraryReset conversion arm`))).toBe(true);
  }
});

test("dropping the StreamId extension line fails the extension assertion", () => {
  const { vendored } = fixture();
  vendored.set("src/server.rs", SERVER_UP);
  const old = parseRecord(vendored.get(".vendored-sha256")).records.get("src/server.rs");
  vendored.set(".vendored-sha256", vendored.get(".vendored-sha256").replace(old.vendored, hashFile("src/server.rs", Buffer.from(SERVER_UP))));
  expect(checkVendor(inputs(vendored)).some((e) => e.includes("does not add the StreamId extension"))).toBe(true);
});

test("PATCHED_FILES names exactly the two files carrying the three changes", () => {
  expect([...PATCHED_FILES].sort()).toEqual(["src/proto/streams/state.rs", "src/server.rs"]);
  expect([...hunkCounts(DIFF)]).toEqual([
    ["src/proto/streams/state.rs", 2],
    ["src/server.rs", 1],
  ]);
});

/** An upstream `.crate` for the fixture, with its pinned hash, and a `readFile` over the vendored tree. */
async function networkedFixture(mutateUpstream = (m) => m) {
  const { upstream, vendored } = fixture();
  const dir = mkdtempSync(join(tmpdir(), "check-vendor-"));
  const tree = mutateUpstream(new Map(upstream));
  tree.set(".cargo-ok", "{}");
  for (const [f, c] of tree) {
    const p = join(dir, "h2-0.4.19", f);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, c, "latin1");
  }
  const file = join(dir, "h2-0.4.19.crate");
  await create({ gzip: true, cwd: dir, file, portable: true }, [...tree.keys()].map((f) => `h2-0.4.19/${f}`));
  const crateBytes = readFileSync(file);
  const { createHash } = await import("node:crypto");
  const files = new Map([...vendored].map(([f, c]) => [`vendor/h2/${f}`, Buffer.from(c, "latin1")]));
  return {
    crateBytes,
    expectedSha256: createHash("sha256").update(crateBytes).digest("hex"),
    readFile: (p) => files.get(p) ?? Buffer.from(""),
    files,
  };
}

test("the networked mode passes against a fixture archive and diff", async () => {
  const f = await networkedFixture();
  expect(checkUpstream(f)).toEqual([]);
});

test("a wrong pinned archive hash fails before anything else", async () => {
  const f = await networkedFixture();
  const errors = checkUpstream({ ...f, expectedSha256: "0".repeat(64) });
  expect(errors).toHaveLength(1);
  expect(errors[0]).toContain("differs from the pinned");
});

test("a one-byte upstream difference fails the recorded upstream hash", async () => {
  const f = await networkedFixture((m) => m.set("src/lib.rs", "pub mod z;\n"));
  expect(checkUpstream(f).some((e) => e.includes("src/lib.rs: recorded upstream hash"))).toBe(true);
});

test("a diff hunk whose context does not match upstream fails", async () => {
  const f = await networkedFixture();
  const key = "vendor/h2/SHADOWCAT-PATCH.diff";
  f.files.set(key, Buffer.from(f.files.get(key).toString("latin1").replace(" match self.inner {", " match other.inner {"), "latin1"));
  expect(checkUpstream(f).some((e) => e.includes("does not match upstream"))).toBe(true);
});

test("one changed byte in the committed diff fails the patched hash", async () => {
  const f = await networkedFixture();
  const key = "vendor/h2/SHADOWCAT-PATCH.diff";
  f.files.set(key, Buffer.from(f.files.get(key).toString("latin1").replace("from_internal", "from_interna1"), "latin1"));
  expect(checkUpstream(f).some((e) => e.includes("hashes to"))).toBe(true);
});

test("applyPatch reproduces the patched fixture text", () => {
  const result = applyPatch(STATE_UP, DIFF, "src/proto/streams/state.rs");
  expect(result).toEqual({ text: STATE_PATCHED });
});

test("runGate reports a git failure instead of throwing", async () => {
  const errors = [];
  const code = await runGate({
    listFiles: () => ({ ok: false, message: "could not determine the vendored file list (boom)" }),
    readFile: () => Buffer.from(""),
    upstream: false,
    log: () => {},
    error: (m) => errors.push(m),
  });
  expect(code).toBe(1);
  expect(errors).toEqual(["lint:vendor: could not determine the vendored file list (boom)"]);
});

test("the real tree passes the offline mode", () => {
  const r = spawnSync(process.execPath, [CLI], { cwd: REPO, encoding: "utf8" });
  expect(r.stderr).toBe("");
  expect(r.status).toBe(0);
});

test("an untracked stray file under vendor/ fails before it is added", () => {
  const dir = mkdtempSync(join(tmpdir(), "check-vendor-git-"));
  const git = (...args) => spawnSync("git", args, { cwd: dir, encoding: "utf8" });
  git("init");
  writeFileSync(join(dir, "Cargo.toml"), ROOT_TOML);
  writeFileSync(join(dir, "Cargo.lock"), LOCK_OK);
  mkdirSync(join(dir, "vendor"));
  writeFileSync(join(dir, "vendor", "notes.rs"), "fn stray() {}\n");
  const r = spawnSync(process.execPath, [CLI], { cwd: dir, encoding: "utf8" });
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("vendor/notes.rs: not under a directory");
});
