import { test, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { findForbiddenCrates, FORBIDDEN_CRATES } from "./check-crypto-backend.mjs";

const CLI = resolve(dirname(fileURLToPath(import.meta.url)), "check-crypto-backend.mjs");

const CLEAN = `[[package]]\nname = "ring"\nversion = "0.17.14"\n\n[[package]]\nname = "rustls"\nversion = "0.23.40"\n`;
const DIRTY = `${CLEAN}\n[[package]]\nname = "aws-lc-rs"\nversion = "1.0.0"\n\n[[package]]\nname = "aws-lc-sys"\nversion = "0.1.0"\n`;

function runIn(lock) {
  const dir = mkdtempSync(join(tmpdir(), "crypto-backend-"));
  writeFileSync(join(dir, "Cargo.lock"), lock);
  return spawnSync(process.execPath, [CLI], { cwd: dir, encoding: "utf8" });
}

test("a lockfile with only ring is clean", () => {
  expect(findForbiddenCrates(CLEAN)).toEqual([]);
});

test("every forbidden crate is detected by exact name", () => {
  for (const name of FORBIDDEN_CRATES) {
    expect(findForbiddenCrates(`[[package]]\nname = "${name}"\nversion = "1.0.0"\n`)).toEqual([name]);
  }
});

test("a crate merely containing the name is not a match", () => {
  expect(findForbiddenCrates(`[[package]]\nname = "aws-lc-rs-compat"\nversion = "1.0.0"\n`)).toEqual([]);
});

test("the CLI exits 1 naming the crates when one is present", () => {
  const r = runIn(DIRTY);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("aws-lc-rs, aws-lc-sys");
});

test("the CLI exits 0 on a clean lockfile", () => {
  expect(runIn(CLEAN).status).toBe(0);
});

test("the CLI accepts the repository's own lockfile (positive control against the real input)", () => {
  const r = spawnSync(process.execPath, [CLI], { cwd: resolve(dirname(CLI), ".."), encoding: "utf8" });
  expect(r.status).toBe(0);
  expect(r.stdout).toContain("in Cargo.lock");
  // The success line is built from the one list, so every forbidden crate is named in it.
  for (const name of FORBIDDEN_CRATES) expect(r.stdout).toContain(name);
});

test("the parser really reads the repository's lockfile: it finds the crate the server pins", () => {
  // A parser that matched nothing would also accept the real lock; finding `ring` proves the
  // detector reads this file's package entries at all.
  const lock = readFileSync(resolve(dirname(CLI), "..", "Cargo.lock"), "utf8");
  expect(findForbiddenCrates(lock, ["ring"])).toEqual(["ring"]);
});

test("a lockfile carrying the OS certificate verifier is rejected", () => {
  const lock = `${CLEAN}\n[[package]]\nname = "rustls-platform-verifier"\nversion = "0.6.0"\n`;
  expect(findForbiddenCrates(lock)).toEqual(["rustls-platform-verifier"]);
  const r = runIn(lock);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("rustls-platform-verifier");
});
