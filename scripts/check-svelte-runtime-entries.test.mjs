import { test, expect } from "vitest";
import { findUnenumeratedSveltePaths } from "./check-svelte-runtime-entries.mjs";

test("flags an svelte/* import not present in RUNTIME_ENTRIES", () => {
  const fakeSourceFiles = {
    "fake/module.ts": `import { onMount } from "svelte";\nimport { fade } from "svelte/transition";\n`,
  };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  const flagged = findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries);

  expect(flagged).toEqual([{ file: "fake/module.ts", specifier: "svelte/transition" }]);
});

test("does not flag an already-enumerated specifier", () => {
  const fakeSourceFiles = { "fake/module.ts": `import { onMount } from "svelte";\n` };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  expect(findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries)).toEqual([]);
});
