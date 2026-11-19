import { defineConfig } from "vitest/config";

// `pnpm run test:scripts` (`vitest run scripts/`) is the intended reader of
// this file. Vitest's config resolution DOES walk up to this file for a
// package that owns no vitest config of its own, and the match against
// `include: ["scripts/**/*.test.mjs"]` from that package's root always
// yields zero tests. What that zero yields next depends on the invoking
// flags: a plain `vitest run` exits 1 ("No test files found, exiting with
// code 1"), but `--passWithNoTests` — the shape every first-party module's
// own test script uses — exits 0 having run nothing, a silent green with no
// assertion ever executed. That silent-pass outcome, not merely the louder
// one, is why every package that runs vitest MUST own its own config.
// `src/client/formula/vitest.config.ts` exists precisely because this file
// is what Vite walked up to when that package had none.
export default defineConfig({
  test: {
    include: ["scripts/**/*.test.mjs"],
    // Many tests spawn real `git` subprocesses, move files to the recycle bin
    // or parse the whole repository (the symbol index, the walk-site scan,
    // the comment-gate corpus) rather than mocking those boundaries, so
    // their wall-clock cost scales with host process-spawn and CPU speed, not
    // with vitest's own per-test overhead. The work is fixed-size. Idle costs,
    // measured with nothing else running: the slowest cases take 1.3 s (the
    // recycle-bin removal test), 2.8 s (a whole-corpus parse) and 6 s (the
    // symbol index beside the other workers). Under heavy host contention
    // (concurrent cargo and rustc builds at 75-100% CPU) the same cases have
    // been observed taking up to about 10x longer; that contention could not
    // be reproduced on demand. The 60 s limit is a sizing margin over that
    // observed 10x, set above the slowest idle case, and it still fails a
    // genuine hang.
    testTimeout: 60000,
    hookTimeout: 60000,
  },
});
