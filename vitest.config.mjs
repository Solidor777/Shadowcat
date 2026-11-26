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
    // Many tests run real `git` and parse the whole repository rather than
    // mocking those boundaries, so their wall-clock cost scales with host load.
    // 60 s sits well above the idle cost of the slowest case and still fails a
    // hang.
    testTimeout: 60000,
    hookTimeout: 60000,
  },
});
