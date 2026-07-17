import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { describe, it, expect } from "vitest";

// .../src/client/shell/src/lib/importMap.test.ts -> repo root is five levels up.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../../..");
const distDir = path.join(repoRoot, "dist");

const RUNTIME_CHUNKS = [
  "svelte",
  "svelte-internal-client",
  "svelte-internal-disclose-version",
  "svelte-reactivity",
  "shadowcat-core",
  "shadowcat-ui-kit",
  "shadowcat-formula",
  "shadowcat-types",
];

describe("shared-runtime import map (build output)", () => {
  // Mirrors embed.rs's `dist_built()` self-skip: this test only means anything
  // after `pnpm --filter @shadowcat/shell build` has run.
  if (!existsSync(path.join(distDir, "index.html"))) {
    it.skip("dist/ not built — run `pnpm --filter @shadowcat/shell build` first", () => {});
    return;
  }

  it("emits a stable-named chunk for every shared runtime", () => {
    for (const name of RUNTIME_CHUNKS) {
      const file = path.join(distDir, "runtime", `${name}.js`);
      expect(existsSync(file), `expected ${file} to exist`).toBe(true);
    }
  });

  it("index.html carries an import map pointing every bare specifier at its chunk", () => {
    const html = readFileSync(path.join(distDir, "index.html"), "utf-8");
    const match = /<script type="importmap">([\s\S]*?)<\/script>/.exec(html);
    expect(match, "no <script type=\"importmap\"> found in dist/index.html").not.toBeNull();
    const map = JSON.parse(match![1]) as { imports: Record<string, string> };
    expect(map.imports["svelte"]).toBe("/runtime/svelte.js");
    expect(map.imports["svelte/internal/client"]).toBe("/runtime/svelte-internal-client.js");
    expect(map.imports["svelte/internal/disclose-version"]).toBe(
      "/runtime/svelte-internal-disclose-version.js",
    );
    expect(map.imports["svelte/reactivity"]).toBe("/runtime/svelte-reactivity.js");
    expect(map.imports["@shadowcat/core"]).toBe("/runtime/shadowcat-core.js");
    expect(map.imports["@shadowcat/ui-kit"]).toBe("/runtime/shadowcat-ui-kit.js");
    expect(map.imports["@shadowcat/formula"]).toBe("/runtime/shadowcat-formula.js");
    expect(map.imports["@shadowcat/types"]).toBe("/runtime/shadowcat-types.js");

    // The import map must precede the app's own module entry script (§3:
    // "injected before any module script executes").
    const mapIdx = html.indexOf('<script type="importmap">');
    const appIdx = html.indexOf('<script type="module"');
    expect(mapIdx).toBeGreaterThanOrEqual(0);
    expect(appIdx).toBeGreaterThan(mapIdx);
  });
});
