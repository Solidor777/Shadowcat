// Enforces the release binary's size budget. Runs identically in CI and locally: the CI workflow
// used to inline this as a `run: |` bash block naming the binary through a GitHub Actions
// expression (`${{ runner.os == 'Windows' && '.exe' || '' }}`). That expression resolves only
// inside Actions, and the gate manifest's whitespace normalisation of a `run: |` block collapses
// its statement separators, so the block is not one command a local tier could ever execute — it
// is unclassifiable as `commit` or `push` by construction. Extracting the check into a script both
// sides call is the fix: the platform branch runs in Node (`process.platform`), portable to every
// runner and to a developer machine alike.
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
