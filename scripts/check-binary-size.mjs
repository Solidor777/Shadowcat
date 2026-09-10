// Enforces the release binary's size budget. Runs identically in CI and locally, as one script
// both sides call: the platform branch runs in Node (`process.platform`), portable to every CI
// runner and to a developer machine alike. A GitHub Actions expression naming the binary (e.g.
// `${{ runner.os == 'Windows' && '.exe' || '' }}`) resolves only inside Actions, and a `run: |`
// bash block is not one command a local tier could ever execute — the gate manifest's whitespace
// normalisation of such a block collapses its statement separators, making it unclassifiable as
// `commit` or `push` by construction. A single portable script sidesteps both constraints at once.
//
// 60 MiB guardrail; tighten as the binary's real baseline settles.

import { statSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

/** The release binary's platform-specific filename — pure, so both branches are directly testable. */
export function binaryNameForPlatform(platform) {
  return platform === "win32" ? "shadowcat.exe" : "shadowcat";
}

export const BINARY_NAME = binaryNameForPlatform(process.platform);
export const BINARY_PATH = join("target", "release", BINARY_NAME);
export const SIZE_LIMIT_BYTES = 62914560;

/** Pure comparison: whether a measured size fits the budget, independent of the filesystem. */
export function checkBinarySize(sizeBytes, limitBytes = SIZE_LIMIT_BYTES) {
  return { ok: sizeBytes < limitBytes, sizeBytes, limitBytes };
}

if (isDirectEntry(import.meta.url)) {
  let size;
  try {
    size = statSync(BINARY_PATH).size;
  } catch (err) {
    if (err.code === "ENOENT") {
      console.error(
        `binary size check: no release binary at ${BINARY_PATH}. Run \`cargo build --release\` first.`,
      );
      process.exit(1);
    }
    throw err;
  }
  console.log(`release binary size: ${size} bytes`);
  const result = checkBinarySize(size);
  if (!result.ok) {
    console.error(
      `binary size ${result.sizeBytes} bytes exceeds the ${result.limitBytes} byte (60 MiB) guardrail; tighten as the binary's real baseline settles.`,
    );
    process.exit(1);
  }
}
