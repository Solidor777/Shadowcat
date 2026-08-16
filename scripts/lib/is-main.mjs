// The single definition of "this module is the script node was asked to run", shared by every
// script that guards a top-level entry block with it.
//
// One definition rather than one per script: the test compares two spellings of the same path, and
// a copy that omits `resolve` disagrees with the others whenever `argv[1]` arrives spelled
// differently from the module URL's own resolution. Its failure mode is a silent no-op — the
// guarded block never runs and the process still exits 0 — so a divergent copy reports success
// while doing nothing, and only comparing the copies could ever reveal it.
//
// Cross-platform: node:path/node:url only.
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

/**
 * Whether the calling module is the process entry point rather than an imported dependency.
 *
 * @param {string} moduleUrl - the caller's own `import.meta.url`.
 * @returns {boolean} true when node was invoked on this module's file.
 */
export function isDirectEntry(moduleUrl) {
  return (
    Boolean(process.argv[1]) &&
    resolve(process.argv[1]) === fileURLToPath(moduleUrl)
  );
}
