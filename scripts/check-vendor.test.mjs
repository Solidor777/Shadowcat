import { test, expect } from "vitest";
import { Buffer } from "node:buffer";
import { spawnSync } from "node:child_process";
import { createServer, get as httpGet } from "node:http";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { create } from "tar";
import { scrubGitEnv } from "./lib/run-git.mjs";
import {
  EXPECTED_HUNKS_BY_FILE,
  PATCHED_FILES,
  applyPatch,
  checkUpstream,
  checkVendor,
  download,
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
const CONN_UP = `impl Connection {
    fn poll2(&mut self, cx: &mut Context) {
        self.poll_ready(cx);
        self.recv_frame();
    }
}
`;
const CONN_PATCHED = CONN_UP.replace(
  "        self.poll_ready(cx);\n",
  "        self.poll_ready(cx);\n        if self.inner.streams.pause_for_accepts() {\n            return Pending;\n        }\n",
);
const COUNTS_UP = `use super::*;

struct Counts {
    open: usize,
}

impl Counts {
    pub fn new() -> Self {
        Counts {
            open: 0,
        }
    }

    pub fn dec(&mut self) {
        self.open -= 1;
    }
}
`;
const COUNTS_PATCHED = COUNTS_UP.replace("use super::*;\n", "use super::*;\nuse std::time::Instant;\n")
  .replace("    open: usize,\n", "    open: usize,\n    remote_reset_rate: ResetRate,\n")
  .replace("            open: 0,\n", "            open: 0,\n            remote_reset_rate: ResetRate::new(),\n")
  .replace(
    "    pub fn dec(&mut self) {",
    "    pub(crate) fn remote_resets_at_bound(&self) -> bool {\n        self.peer.is_server() && self.num_remote_reset_streams >= self.max_remote_reset_streams\n    }\n\n    pub fn dec(&mut self) {",
  )
  .replace(
    "        self.open -= 1;\n    }\n",
    "        self.open -= 1;\n    }\n\n    pub(crate) fn record_remote_reset(&mut self) {\n        self.remote_reset_rate.record(Instant::now());\n    }\n",
  );
const RECV_UP = `impl Recv {
    pub fn recv_reset(&mut self, counts: &mut Counts) {
        self.notify();
    }
}
`;
const RECV_PATCHED = RECV_UP.replace(
  "        self.notify();\n",
  "        if counts.record_remote_reset().is_err() {\n            return;\n        }\n        self.notify();\n",
);
const STREAMS_UP = `impl Streams {
    pub fn next_incoming(&mut self) {
        let key = pop();
        key
    }
}
`;
const STREAMS_PATCHED = STREAMS_UP.replace(
  "    pub fn next_incoming(&mut self) {",
  "    pub fn pause_for_accepts(&self) -> bool {\n        me.counts.remote_resets_at_bound()\n    }\n\n    pub fn next_incoming(&mut self) {",
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
  sectionFor("src/server.rs", SERVER_UP, SERVER_PATCHED) +
  sectionFor("src/proto/connection.rs", CONN_UP, CONN_PATCHED) +
  sectionFor("src/proto/streams/counts.rs", COUNTS_UP, COUNTS_PATCHED) +
  sectionFor("src/proto/streams/recv.rs", RECV_UP, RECV_PATCHED) +
  sectionFor("src/proto/streams/streams.rs", STREAMS_UP, STREAMS_PATCHED);

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
    ["src/proto/connection.rs", CONN_UP],
    ["src/proto/streams/counts.rs", COUNTS_UP],
    ["src/proto/streams/recv.rs", RECV_UP],
    ["src/proto/streams/streams.rs", STREAMS_UP],
    ["data.bin", "a\r\nb"],
  ]);
  const vendored = new Map(upstream);
  vendored.set("src/proto/streams/state.rs", STATE_PATCHED);
  vendored.set("src/server.rs", SERVER_PATCHED);
  vendored.set("src/proto/connection.rs", CONN_PATCHED);
  vendored.set("src/proto/streams/counts.rs", COUNTS_PATCHED);
  vendored.set("src/proto/streams/recv.rs", RECV_PATCHED);
  vendored.set("src/proto/streams/streams.rs", STREAMS_PATCHED);
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

test("PATCHED_FILES names exactly the six files carrying the vendored changes", () => {
  expect([...PATCHED_FILES].sort()).toEqual([
    "src/proto/connection.rs",
    "src/proto/streams/counts.rs",
    "src/proto/streams/recv.rs",
    "src/proto/streams/state.rs",
    "src/proto/streams/streams.rs",
    "src/server.rs",
  ]);
  expect(Object.fromEntries(hunkCounts(DIFF))).toEqual(EXPECTED_HUNKS_BY_FILE);
});

/** Replaces a vendored file's text and fixes its record so only the behaviour assertions can object. */
function withFile(vendored, file, text) {
  vendored.set(file, text);
  const old = parseRecord(vendored.get(".vendored-sha256")).records.get(file);
  vendored.set(".vendored-sha256", vendored.get(".vendored-sha256").replace(old.vendored, hashFile(file, Buffer.from(text))));
}

test("reverting any pending-accept reset change fails its assertion even when its record is updated", () => {
  const cases = [
    ["src/proto/connection.rs", CONN_UP, "does not stop reading while pending-accept resets are at their bound"],
    ["src/proto/streams/recv.rs", RECV_UP, "does not enforce the sustained reset-rate limit"],
    ["src/proto/streams/counts.rs", COUNTS_UP, "does not compare the count with the bound"],
    ["src/proto/streams/counts.rs", COUNTS_UP, "does not charge the reset-rate limit"],
    ["src/proto/streams/streams.rs", STREAMS_UP, "does not test the pending-accept reset bound"],
  ];
  for (const [file, upstream, message] of cases) {
    const { vendored } = fixture();
    withFile(vendored, file, upstream);
    expect(checkVendor(inputs(vendored)).some((e) => e.includes(message)), `${file}: ${message}`).toBe(true);
  }
});

test("a tampered backpressure hunk in the vendored file fails its record hash", () => {
  const { vendored } = fixture();
  vendored.set("src/proto/connection.rs", CONN_PATCHED.replace("pause_for_accepts()", "pause_for_accept()"));
  expect(checkVendor(inputs(vendored)).some((e) => e.includes("src/proto/connection.rs: hash"))).toBe(true);
});

test("a tampered backpressure hunk in the committed diff fails the upstream replay", async () => {
  const f = await networkedFixture();
  const key = "vendor/h2/SHADOWCAT-PATCH.diff";
  f.files.set(key, Buffer.from(f.files.get(key).toString("latin1").replace("pause_for_accepts()", "pause_for_accept()"), "latin1"));
  expect(checkUpstream(f).some((e) => e.includes("src/proto/connection.rs: upstream plus"))).toBe(true);
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

test("a failed download is reported as a lint:vendor line and exit code 1, not thrown", async () => {
  const errors = [];
  const code = await runGate({
    listFiles: () => ({ ok: true, paths: [] }),
    readFile: () => Buffer.from(""),
    upstream: true,
    fetchCrate: () => Promise.reject(new Error("getaddrinfo ENOTFOUND static.crates.io")),
    log: () => {},
    error: (m) => errors.push(m),
  });
  expect(code).toBe(1);
  expect(errors.some((m) => m === "lint:vendor: could not download the upstream archive: getaddrinfo ENOTFOUND static.crates.io")).toBe(true);
});

test("a failed download beside an offline defect prints both", async () => {
  const { vendored } = fixture();
  const sameInputs = () => inputs(vendored, { extraPaths: ["vendor/notes.rs"] });
  const offline = checkVendor(sameInputs());
  expect(offline.length).toBeGreaterThan(0);
  const { paths, readFile, rootCargoToml, cargoLock } = sameInputs();
  const errors = [];
  const code = await runGate({
    listFiles: () => ({ ok: true, paths }),
    readFile: (p) => (p === "Cargo.toml" ? Buffer.from(rootCargoToml) : p === "Cargo.lock" ? Buffer.from(cargoLock) : readFile(p)),
    upstream: true,
    fetchCrate: () => Promise.reject(new Error("getaddrinfo ENOTFOUND static.crates.io")),
    log: () => {},
    error: (m) => errors.push(m),
  });
  expect(code).toBe(1);
  for (const line of offline) expect(errors).toContain(`lint:vendor: ${line}`);
  expect(errors).toContain("lint:vendor: could not download the upstream archive: getaddrinfo ENOTFOUND static.crates.io");
  expect(errors.length).toBe(offline.length + 1);
});

test("a redirect with a malformed Location rejects with a message instead of throwing", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(302, { location: "http://[::1" });
    res.end();
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    await expect(download(`http://127.0.0.1:${port}/h2.crate`, { request: httpGet })).rejects.toThrow(/Location/);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("a server that never answers ends the download at its deadline", async () => {
  const server = createServer(() => {});
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    const started = Date.now();
    await expect(download(`http://127.0.0.1:${port}/h2.crate`, { deadlineMs: 150, request: httpGet })).rejects.toThrow(
      /did not finish within 150 ms/,
    );
    expect(Date.now() - started).toBeLessThan(5000);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("a server that stops sending mid-body ends the download at its deadline", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(200, { "content-length": "1000" });
    res.write("partial");
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    await expect(download(`http://127.0.0.1:${port}/h2.crate`, { deadlineMs: 150, request: httpGet })).rejects.toThrow();
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("a redirect to a plain-HTTP location is refused", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(302, { location: "http://127.0.0.1:1/h2.crate" });
    res.end();
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    await expect(download(`http://127.0.0.1:${port}/h2.crate`, { request: httpGet })).rejects.toThrow(/not HTTPS/);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("a non-200 answer rejects with its status", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(503);
    res.end();
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    await expect(download(`http://127.0.0.1:${port}/h2.crate`, { request: httpGet })).rejects.toThrow(/HTTP 503/);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("a body that arrives whole resolves with its bytes", async () => {
  const server = createServer((_req, res) => {
    res.writeHead(200);
    res.end("archive-bytes");
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const { port } = server.address();
    const got = await download(`http://127.0.0.1:${port}/h2.crate`, { request: httpGet });
    expect(got.toString()).toBe("archive-bytes");
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("the real tree passes the offline mode", () => {
  const r = spawnSync(process.execPath, [CLI], { cwd: REPO, encoding: "utf8" });
  expect(r.stderr).toBe("");
  expect(r.status).toBe(0);
});

test("an untracked stray file under vendor/ fails before it is added", () => {
  const dir = mkdtempSync(join(tmpdir(), "check-vendor-git-"));
  // The test process runs inside `git commit`'s hook, which exports GIT_DIR; from a linked worktree
  // an unscrubbed `git init` re-initialises that worktree's real git directory and flips the shared
  // `core.bare` to true. Both spawns therefore start from `scrubGitEnv()`.
  const env = scrubGitEnv();
  const git = (...args) => spawnSync("git", args, { cwd: dir, encoding: "utf8", env });
  git("init");
  writeFileSync(join(dir, "Cargo.toml"), ROOT_TOML);
  writeFileSync(join(dir, "Cargo.lock"), LOCK_OK);
  mkdirSync(join(dir, "vendor"));
  writeFileSync(join(dir, "vendor", "notes.rs"), "fn stray() {}\n");
  const r = spawnSync(process.execPath, [CLI], { cwd: dir, encoding: "utf8", env });
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("vendor/notes.rs: not under a directory");
});
