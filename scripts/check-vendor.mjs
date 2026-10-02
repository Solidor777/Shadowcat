// Fails when project-authored content hides under `vendor/` or a vendored crate drifts from its
// recorded upstream plus the committed patch.
//
// `vendor/` is outside every other gate's roots by absence (file-size, comment, allowance and
// inline-test gates enumerate `src`, `scripts` and `examples`; `cargo fmt`, `clippy` and the
// doctests never reach a patched dependency that is not a workspace member), so this gate is what
// stops a file placed there from escaping all of them. Two modes of one script:
//
//   offline (default)  reads `vendor/`, the root `Cargo.toml` and `Cargo.lock`; needs no network
//                      and no toolchain, so it runs on every runner and in the commit hook.
//   `--upstream`       additionally downloads the pinned upstream `.crate`, checks its SHA-256,
//                      and proves the recorded upstream hashes are real and the only delta is the
//                      committed `SHADOWCAT-PATCH.diff`. The archive is read in memory: nothing is
//                      extracted to disk.
//
// INVARIANT: every file under `vendor/<crate>/` has exactly one record in `.vendored-sha256`
// (`path  upstream_sha256  vendored_sha256`), except the two prose/record files named in
// `RECORD_EXEMPT`. The records whose two hashes differ are exactly `PATCHED_FILES`; a file the patch
// adds has `-` as its upstream hash and is listed in `ADDED_FILES`.

import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { get } from "node:https";
import { URL } from "node:url";
import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import process from "node:process";
import { parse as parseToml } from "smol-toml";
import { Parser } from "tar";
import { isDirectEntry } from "./lib/is-main.mjs";
import { runGit } from "./lib/run-git.mjs";

/** Crate directories `vendor/` may contain; any other path under `vendor/` fails. */
export const VENDORED_CRATES = ["h2"];

/** The crate version vendored, and the SHA-256 of its upstream `.crate` archive. */
export const UPSTREAM = {
  name: "h2",
  version: "0.4.19",
  sha256: "ef8e5e5a340588f4452631496976cf8636d4a7ecf600239fdc27615d2530bc16",
  url: "https://static.crates.io/crates/h2/h2-0.4.19.crate",
};

/**
 * The vendored files that differ from upstream. They carry the connection-end arms
 * (`State::recv_eof`, `State::handle_error`), the `h2::StreamId` request extension
 * (`Peer::convert_poll_message`), and the pending-accept reset backpressure with the sustained
 * reset-rate rule (`Connection::poll2`, `Streams::pause_for_accepts`, `ResetRate`).
 */
export const PATCHED_FILES = [
  "src/proto/connection.rs",
  "src/proto/streams/counts.rs",
  "src/proto/streams/recv.rs",
  "src/proto/streams/state.rs",
  "src/proto/streams/streams.rs",
  "src/server.rs",
];

/** Files the patch adds beside upstream's tree; their record's upstream hash is `-`. */
export const ADDED_FILES = ["SHADOWCAT-PATCH.diff"];

/** Files with no record: the record itself and the prose describing the patch. */
export const RECORD_EXEMPT = [".vendored-sha256", "SHADOWCAT-PATCH.md"];

/** The committed unified diff of `PATCHED_FILES`, and the record file, relative to the crate. */
export const PATCH_DIFF = "SHADOWCAT-PATCH.diff";
export const RECORD_FILE = ".vendored-sha256";

/** Upstream tarball entries the vendored tree omits: tarball metadata that is not source. */
export const REMOVED_UPSTREAM = [".cargo-ok", ".cargo_vcs_info.json", ".gitignore", "Cargo.lock"];
export const REMOVED_UPSTREAM_PREFIXES = [".github/"];

/** Hunks the committed diff must carry in each patched file. */
export const EXPECTED_HUNKS_BY_FILE = {
  "src/proto/connection.rs": 1,
  "src/proto/streams/counts.rs": 5,
  "src/proto/streams/recv.rs": 1,
  "src/proto/streams/state.rs": 2,
  "src/proto/streams/streams.rs": 1,
  "src/server.rs": 1,
};

/** Number of hunks the committed diff must carry in all. */
export const EXPECTED_HUNKS = Object.values(EXPECTED_HUNKS_BY_FILE).reduce((a, b) => a + b, 0);

/**
 * What each vendored change must still contain, so a hunk reverted together with its record is
 * caught: the function whose body must include the text, and the failure message.
 */
export const BEHAVIOUR_CHECKS = [
  { file: "src/proto/streams/state.rs", fn: "recv_eof", needle: "Closed(Cause::ScheduledLibraryReset(_)) =>", message: "State::recv_eof has no ScheduledLibraryReset conversion arm" },
  { file: "src/proto/streams/state.rs", fn: "handle_error", needle: "Closed(Cause::ScheduledLibraryReset(_)) =>", message: "State::handle_error has no ScheduledLibraryReset conversion arm" },
  { file: "src/server.rs", fn: "convert_poll_message", needle: "crate::StreamId::from_internal(stream_id)", message: "Peer::convert_poll_message does not add the StreamId extension" },
  { file: "src/proto/connection.rs", fn: "poll2", needle: "pause_for_accepts()", message: "Connection::poll2 does not stop reading while pending-accept resets are at their bound" },
  { file: "src/proto/streams/streams.rs", fn: "pause_for_accepts", needle: "remote_resets_at_bound()", message: "Streams::pause_for_accepts does not test the pending-accept reset bound" },
  { file: "src/proto/streams/counts.rs", fn: "remote_resets_at_bound", needle: "self.num_remote_reset_streams >= self.max_remote_reset_streams", message: "Counts::remote_resets_at_bound does not compare the count with the bound" },
  { file: "src/proto/streams/counts.rs", fn: "record_remote_reset", needle: "self.remote_reset_rate.record(", message: "Counts::record_remote_reset does not charge the reset-rate limit" },
  { file: "src/proto/streams/recv.rs", fn: "recv_reset", needle: "counts.record_remote_reset()", message: "Recv::recv_reset does not enforce the sustained reset-rate limit" },
];

const TEXT_EXTENSIONS = [".rs", ".toml", ".md", ".orig"];

/**
 * SHA-256 of a vendored file, with CRLF folded to LF for text files only so a Windows checkout
 * hashes like the LF the repository stores.
 * @param {string} relPath Path relative to the crate directory.
 * @param {Buffer} bytes The file contents.
 * @returns {string} Lowercase hex digest.
 */
export function hashFile(relPath, bytes) {
  const leaf = relPath.split("/").pop();
  const isText = TEXT_EXTENSIONS.some((e) => leaf.endsWith(e)) || leaf === "LICENSE";
  const data = isText ? Buffer.from(bytes.toString("latin1").replace(/\r\n/g, "\n"), "latin1") : bytes;
  return createHash("sha256").update(data).digest("hex");
}

/**
 * Parses `.vendored-sha256`: one `path  upstream  vendored` record per line.
 * @param {string} text The record file contents.
 * @returns {{ records: Map<string, {upstream: string, vendored: string}>, errors: string[] }} Records and parse errors.
 */
export function parseRecord(text) {
  const records = new Map();
  const errors = [];
  for (const [i, line] of text.split(/\r?\n/).entries()) {
    if (line.trim() === "") continue;
    const m = line.match(/^(\S+) {2}([0-9a-f]{64}|-) {2}([0-9a-f]{64})$/);
    if (!m) {
      errors.push(`${RECORD_FILE}:${i + 1}: cannot parse "${line}"`);
    } else if (records.has(m[1])) {
      errors.push(`${RECORD_FILE}:${i + 1}: duplicate record for ${m[1]}`);
    } else {
      records.set(m[1], { upstream: m[2], vendored: m[3] });
    }
  }
  return { records, errors };
}

/**
 * Body of the first `fn <name>` in `source`, up to the next function at the same indent.
 * @param {string} source Rust source.
 * @param {string} name Function name.
 * @returns {string} The text from the signature to the next `    pub fn`/`    fn` line, or "".
 */
export function functionBody(source, name) {
  const start = source.search(new RegExp(`^ {4}(?:pub(?:\\(crate\\))? )?fn ${name}\\b`, "m"));
  if (start < 0) return "";
  const rest = source.slice(start + 1);
  const next = rest.search(/^ {4}(?:pub(?:\(crate\))? )?fn /m);
  return next < 0 ? rest : rest.slice(0, next);
}

/**
 * Number of `@@` hunks per file in a unified diff.
 * @param {string} diff The diff text.
 * @returns {Map<string, number>} File path (as `b/` names it, without the prefix) to hunk count.
 */
export function hunkCounts(diff) {
  const counts = new Map();
  let file = null;
  for (const line of diff.split("\n")) {
    const f = line.match(/^\+\+\+ b\/(\S+)/);
    if (f) {
      file = f[1];
      counts.set(file, 0);
    } else if (line.startsWith("@@") && file !== null) {
      counts.set(file, counts.get(file) + 1);
    }
  }
  return counts;
}

/**
 * Applies the hunks of a unified diff for one file to `original`, verifying every context and
 * removed line against it.
 * @param {string} original The upstream file text (LF).
 * @param {string} diff The whole diff text.
 * @param {string} file The file's path as the diff names it.
 * @returns {{ text: string } | { error: string }} The patched text, or why a hunk does not apply.
 */
export function applyPatch(original, diff, file) {
  const lines = diff.split("\n");
  const begin = lines.findIndex((l) => l === `+++ b/${file}`);
  if (begin < 0) return { error: `no diff section for ${file}` };
  const src = original.split("\n");
  const out = [];
  let cursor = 0;
  let i = begin + 1;
  while (i < lines.length && !lines[i].startsWith("--- ")) {
    const h = lines[i].match(/^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@/);
    if (!h) {
      i++;
      continue;
    }
    const start = Number(h[1]) - 1;
    if (start < cursor) return { error: `${file}: hunks overlap or are out of order` };
    out.push(...src.slice(cursor, start));
    cursor = start;
    i++;
    while (i < lines.length && !lines[i].startsWith("@@") && !lines[i].startsWith("--- ")) {
      const l = lines[i];
      const tag = l[0];
      const body = l.slice(1);
      if (tag === " " || tag === "-") {
        if (src[cursor] !== body) {
          return { error: `${file}: hunk at line ${start + 1} does not match upstream (line ${cursor + 1})` };
        }
        if (tag === " ") out.push(body);
        cursor++;
      } else if (tag === "+") {
        out.push(body);
      } else if (l !== "" && tag !== "\\") {
        return { error: `${file}: unreadable diff line "${l}"` };
      }
      i++;
    }
  }
  out.push(...src.slice(cursor));
  return { text: out.join("\n") };
}

/**
 * Offline checks over the vendored tree and the two cargo files.
 * @param {object} input
 * @param {string[]} input.paths Every path under `vendor/`, repo-relative with forward slashes.
 * @param {(path: string) => Buffer} input.readFile Reads a repo-relative path.
 * @param {string} input.rootCargoToml Root `Cargo.toml` text.
 * @param {string} input.cargoLock `Cargo.lock` text.
 * @returns {string[]} One message per failure; empty when the tree is sound.
 */
export function checkVendor({ paths, readFile, rootCargoToml, cargoLock }) {
  const errors = [];
  const crateDirs = VENDORED_CRATES.map((c) => `vendor/${c}/`);

  for (const p of paths) {
    if (!crateDirs.some((d) => p.startsWith(d))) {
      errors.push(`${p}: not under a directory named in VENDORED_CRATES (${VENDORED_CRATES.join(", ")})`);
    }
  }

  const lock = parseToml(cargoLock);
  const h2Packages = (lock.package ?? []).filter((p) => p.name === UPSTREAM.name);
  if (h2Packages.length !== 1) {
    errors.push(`Cargo.lock: expected exactly one ${UPSTREAM.name} package, found ${h2Packages.length}`);
  } else if (h2Packages[0].source !== undefined) {
    errors.push(`Cargo.lock: ${UPSTREAM.name} has a source (${h2Packages[0].source}), so the [patch.crates-io] path is not in effect`);
  }

  const crateDir = `vendor/${UPSTREAM.name}/`;
  const crateFiles = paths.filter((p) => p.startsWith(crateDir)).map((p) => p.slice(crateDir.length));
  const has = (rel) => crateFiles.includes(rel);

  if (!has("Cargo.toml")) {
    errors.push(`${crateDir}Cargo.toml: missing`);
  } else {
    const version = parseToml(readFile(`${crateDir}Cargo.toml`).toString("utf8")).package?.version;
    if (h2Packages.length === 1 && h2Packages[0].version !== version) {
      errors.push(`${crateDir}Cargo.toml version ${version} differs from Cargo.lock ${UPSTREAM.name} ${h2Packages[0].version}`);
    }
    if (version !== UPSTREAM.version) {
      errors.push(`${crateDir}Cargo.toml version ${version} differs from the pinned upstream ${UPSTREAM.version}`);
    }
  }

  if (!has("LICENSE")) errors.push(`${crateDir}LICENSE: missing`);
  else if (!/Permission is hereby granted, free of charge/.test(readFile(`${crateDir}LICENSE`).toString("utf8"))) {
    errors.push(`${crateDir}LICENSE: does not contain the MIT licence text`);
  }

  const root = parseToml(rootCargoToml);
  if (!(root.workspace?.exclude ?? []).includes(crateDir.slice(0, -1))) {
    errors.push(`Cargo.toml: [workspace] exclude does not list ${crateDir.slice(0, -1)}`);
  }
  if (root.patch?.["crates-io"]?.[UPSTREAM.name]?.path !== crateDir.slice(0, -1)) {
    errors.push(`Cargo.toml: [patch.crates-io] does not map ${UPSTREAM.name} to ${crateDir.slice(0, -1)}`);
  }

  if (!has(RECORD_FILE)) {
    errors.push(`${crateDir}${RECORD_FILE}: missing`);
    return errors;
  }
  const { records, errors: recordErrors } = parseRecord(readFile(`${crateDir}${RECORD_FILE}`).toString("utf8"));
  errors.push(...recordErrors);
  const expectedRecorded = crateFiles.filter((f) => !RECORD_EXEMPT.includes(f));
  for (const f of expectedRecorded) {
    const rec = records.get(f);
    if (rec === undefined) {
      errors.push(`${crateDir}${f}: no record in ${RECORD_FILE}`);
      continue;
    }
    const actual = hashFile(f, readFile(`${crateDir}${f}`));
    if (actual !== rec.vendored) errors.push(`${crateDir}${f}: hash ${actual} differs from its record ${rec.vendored}`);
  }
  for (const f of records.keys()) {
    if (!expectedRecorded.includes(f)) errors.push(`${RECORD_FILE}: record for ${f} has no file`);
  }
  const differing = [...records].filter(([, r]) => r.upstream !== "-" && r.upstream !== r.vendored).map(([f]) => f).sort();
  if (JSON.stringify(differing) !== JSON.stringify([...PATCHED_FILES].sort())) {
    errors.push(`records that differ from upstream are [${differing.join(", ")}]; PATCHED_FILES is [${PATCHED_FILES.join(", ")}]`);
  }
  const added = [...records].filter(([, r]) => r.upstream === "-").map(([f]) => f).sort();
  if (JSON.stringify(added) !== JSON.stringify([...ADDED_FILES].sort())) {
    errors.push(`records without an upstream hash are [${added.join(", ")}]; ADDED_FILES is [${ADDED_FILES.join(", ")}]`);
  }

  if (has(PATCH_DIFF)) {
    const counts = hunkCounts(readFile(`${crateDir}${PATCH_DIFF}`).toString("utf8"));
    for (const [file, n] of Object.entries(EXPECTED_HUNKS_BY_FILE)) {
      if (counts.get(file) !== n) errors.push(`${PATCH_DIFF}: ${file} carries ${counts.get(file) ?? 0} hunk(s), expected ${n}`);
    }
    const total = [...counts.values()].reduce((a, b) => a + b, 0);
    if (total !== EXPECTED_HUNKS) errors.push(`${PATCH_DIFF}: ${total} hunk(s), expected ${EXPECTED_HUNKS}`);
  }

  for (const { file, fn, needle, message } of BEHAVIOUR_CHECKS) {
    if (!has(file)) continue;
    const text = readFile(`${crateDir}${file}`).toString("utf8").replace(/\r\n/g, "\n");
    if (!functionBody(text, fn).includes(needle)) errors.push(`${crateDir}${file}: ${message}`);
  }
  return errors;
}

/**
 * Reads an upstream `.crate` (gzip of tar) in memory.
 * @param {Buffer} crateBytes The downloaded archive.
 * @returns {Map<string, Buffer>} Entry path (without the `<name>-<version>/` prefix) to its bytes.
 */
export function readCrateArchive(crateBytes) {
  const entries = new Map();
  const parser = new Parser({
    sync: true,
    onReadEntry: (entry) => {
      if (entry.type !== "File") return;
      const chunks = [];
      entry.on("data", (c) => chunks.push(c));
      entry.on("end", () => entries.set(entry.path.replace(/^[^/]+\//, ""), Buffer.concat(chunks)));
    },
  });
  parser.write(gunzipSync(crateBytes));
  parser.end();
  return entries;
}

/**
 * Networked checks: the upstream archive matches its pinned hash, every recorded upstream hash is
 * real, the vendored file set is the upstream set minus the removed metadata plus the added
 * files, and applying the committed diff to upstream reproduces each patched file.
 * @param {object} input
 * @param {Buffer} input.crateBytes The downloaded upstream archive.
 * @param {(path: string) => Buffer} input.readFile Reads a repo-relative path.
 * @param {string} [input.expectedSha256] The pinned archive hash; a test fixture supplies its own.
 * @returns {string[]} One message per failure.
 */
export function checkUpstream({ crateBytes, readFile, expectedSha256 = UPSTREAM.sha256 }) {
  const errors = [];
  const archiveHash = createHash("sha256").update(crateBytes).digest("hex");
  if (archiveHash !== expectedSha256) {
    return [`upstream archive hash ${archiveHash} differs from the pinned ${expectedSha256}`];
  }
  const upstream = readCrateArchive(crateBytes);
  const crateDir = `vendor/${UPSTREAM.name}/`;
  const { records } = parseRecord(readFile(`${crateDir}${RECORD_FILE}`).toString("utf8"));

  const kept = [...upstream.keys()].filter(
    (p) => !REMOVED_UPSTREAM.includes(p) && !REMOVED_UPSTREAM_PREFIXES.some((x) => p.startsWith(x)),
  );
  for (const p of kept) {
    const rec = records.get(p);
    if (rec === undefined) {
      errors.push(`upstream file ${p} has no record`);
      continue;
    }
    const upstreamHash = hashFile(p, upstream.get(p));
    if (rec.upstream !== upstreamHash) errors.push(`${p}: recorded upstream hash ${rec.upstream} differs from upstream ${upstreamHash}`);
  }
  for (const [p, rec] of records) {
    if (rec.upstream !== "-" && !kept.includes(p)) errors.push(`${RECORD_FILE}: ${p} is not in the upstream archive`);
  }

  const diff = readFile(`${crateDir}${PATCH_DIFF}`).toString("utf8");
  for (const f of PATCHED_FILES) {
    const original = upstream.get(f);
    if (original === undefined) {
      errors.push(`${f}: not in the upstream archive`);
      continue;
    }
    const patched = applyPatch(original.toString("latin1").replace(/\r\n/g, "\n"), diff, f);
    if ("error" in patched) {
      errors.push(`${PATCH_DIFF}: ${patched.error}`);
      continue;
    }
    const got = hashFile(f, Buffer.from(patched.text, "latin1"));
    const want = records.get(f)?.vendored;
    if (got !== want) errors.push(`${f}: upstream plus ${PATCH_DIFF} hashes to ${got}, the vendored record is ${want}`);
  }
  return errors;
}

/**
 * Lists the files under `vendor/`: tracked plus untracked non-ignored, so a stray file fails
 * before it is added.
 * @returns {{ok: true, paths: string[]} | {ok: false, message: string}} The paths or the git failure.
 */
function listVendorPaths() {
  const r = runGit(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "vendor"], "the vendored file list");
  if (!r.ok) return r;
  return { ok: true, paths: [...new Set(r.stdout.split("\0").filter(Boolean))].sort() };
}

/**
 * Downloads `url` over HTTPS, following up to five redirects.
 * @param {string} url The archive URL.
 * @param {number} [redirects] Redirects still allowed.
 * @returns {Promise<Buffer>} The response body.
 */
function download(url, redirects = 5) {
  return new Promise((resolvePromise, reject) => {
    get(url, (response) => {
      const { statusCode, headers } = response;
      if (statusCode >= 300 && statusCode < 400 && headers.location && redirects > 0) {
        response.resume();
        resolvePromise(download(new URL(headers.location, url).href, redirects - 1));
      } else if (statusCode !== 200) {
        response.resume();
        reject(new Error(`lint:vendor: downloading ${url} failed with HTTP ${statusCode}`));
      } else {
        const chunks = [];
        response.on("data", (c) => chunks.push(c));
        response.on("end", () => resolvePromise(Buffer.concat(chunks)));
        response.on("error", reject);
      }
    }).on("error", reject);
  });
}

/**
 * Entry point shared by the CLI and the tests.
 * @param {object} deps Injected effects.
 * @param {() => {ok: boolean, paths?: string[], message?: string}} deps.listFiles Vendored paths.
 * @param {(path: string) => Buffer} deps.readFile Reads a repo-relative path.
 * @param {boolean} deps.upstream Whether to run the networked checks.
 * @param {() => Promise<Buffer>} [deps.fetchCrate] Downloads the upstream archive.
 * @param {(m: string) => void} deps.log Info sink.
 * @param {(m: string) => void} deps.error Error sink.
 * @returns {Promise<number>} Process exit code.
 */
export async function runGate({ listFiles, readFile, upstream, fetchCrate, log, error }) {
  const listed = listFiles();
  if (!listed.ok) {
    error(`lint:vendor: ${listed.message}`);
    return 1;
  }
  const errors = checkVendor({
    paths: listed.paths,
    readFile,
    rootCargoToml: readFile("Cargo.toml").toString("utf8"),
    cargoLock: readFile("Cargo.lock").toString("utf8"),
  });
  if (upstream) {
    errors.push(...checkUpstream({ crateBytes: await fetchCrate(), readFile }));
  }
  if (errors.length > 0) {
    for (const e of errors) error(`lint:vendor: ${e}`);
    return 1;
  }
  log(`lint:vendor: ${listed.paths.length} vendored file(s), ${PATCHED_FILES.length} patched, ${upstream ? "upstream verified" : "offline"}, 0 error(s)`);
  return 0;
}

if (isDirectEntry(import.meta.url)) {
  const code = await runGate({
    listFiles: listVendorPaths,
    readFile: (p) => readFileSync(p),
    upstream: process.argv.includes("--upstream"),
    fetchCrate: () => download(UPSTREAM.url),
    log: (m) => console.log(m),
    error: (m) => console.error(m),
  });
  process.exit(code);
}
