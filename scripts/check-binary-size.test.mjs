import { test, expect } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { checkBinarySize, SIZE_LIMIT_BYTES, binaryNameForPlatform } from "./check-binary-size.mjs";

const CLI = resolve(dirname(fileURLToPath(import.meta.url)), "check-binary-size.mjs");

test("a size under the budget passes", () => {
  expect(checkBinarySize(SIZE_LIMIT_BYTES - 1)).toEqual({
    ok: true,
    sizeBytes: SIZE_LIMIT_BYTES - 1,
    limitBytes: SIZE_LIMIT_BYTES,
  });
});

test("a size at or over the budget fails", () => {
  expect(checkBinarySize(SIZE_LIMIT_BYTES).ok).toBe(false);
  expect(checkBinarySize(SIZE_LIMIT_BYTES + 1).ok).toBe(false);
});

test("a custom limit is honoured instead of the default", () => {
  expect(checkBinarySize(100, 200)).toEqual({ ok: true, sizeBytes: 100, limitBytes: 200 });
  expect(checkBinarySize(300, 200).ok).toBe(false);
});

test("the Windows platform gets the .exe name", () => {
  expect(binaryNameForPlatform("win32")).toBe("shadowcat.exe");
});

test("every non-Windows platform gets the extensionless name", () => {
  expect(binaryNameForPlatform("linux")).toBe("shadowcat");
  expect(binaryNameForPlatform("darwin")).toBe("shadowcat");
});

test("a missing release binary fails with a message naming the expected path and the fix, not a raw stack trace", () => {
  const emptyDir = mkdtempSync(join(tmpdir(), "no-release-binary-"));
  const result = spawnSync(process.execPath, [CLI], { cwd: emptyDir, encoding: "utf8" });
  expect(result.status).not.toBe(0);
  expect(result.stderr).toMatch(/target[/\\]release/);
  expect(result.stderr).toMatch(/cargo build --release/);
  expect(result.stderr).not.toMatch(/ENOENT/);
});
