import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import { describe, it, expect } from "vitest";

// This module's directory sits five levels below the repo root.
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
  // Mirrors `dist_built()`'s self-skip: this test only means anything
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

    // The import map must precede the app's own module entry script: it must be injected
    // before any module script executes.
    const mapIdx = html.indexOf('<script type="importmap">');
    const appIdx = html.indexOf('<script type="module"');
    expect(mapIdx).toBeGreaterThanOrEqual(0);
    expect(appIdx).toBeGreaterThan(mapIdx);
  });

  // Single-instance sharing — every consumer of a shared package resolves to one runtime copy
  // of it — takes more than a stable chunk name: Vite's default
  // `preserveEntrySignatures: false` lets Rollup drop or mangle an entry chunk's exports down
  // to whatever THIS build's own module graph happens to consume by name. That ships a chunk
  // file at the right path with the wrong (or zero) export bindings, so an external module's
  // `import { mount } from "svelte"` resolves through the import map but throws
  // `SyntaxError: does not provide an export named 'mount'` at runtime — invisible to a test
  // that only checks file existence / the import map's string content. These assertions import
  // the actual built chunks and check the real public API names survive.
  // Imports six real built chunks from disk — I/O-bound, so the default 5s
  // budget flakes under full-workspace parallel runs (`pnpm -r test`).
  it("runtime chunks export their packages' real public API (not mangled aliases)", { timeout: 30_000 }, async () => {
    const svelte = await import(
      pathToFileURL(path.join(distDir, "runtime", "svelte.js")).href
    );
    expect(typeof svelte.mount).toBe("function");
    expect(typeof svelte.tick).toBe("function");
    expect(typeof svelte.unmount).toBe("function");

    // svelte/internal/client is the compiler-emitted import every compiled
    // Svelte 5 component uses (`push`, `pop`, `template`, ...); an empty or
    // near-empty export set means the chunk was tree-shaken to nothing.
    const svelteInternalClient = await import(
      pathToFileURL(path.join(distDir, "runtime", "svelte-internal-client.js")).href
    );
    expect(Object.keys(svelteInternalClient).length).toBeGreaterThan(10);
    expect(typeof svelteInternalClient.push).toBe("function");
    expect(typeof svelteInternalClient.pop).toBe("function");
    expect(typeof svelteInternalClient.template_effect).toBe("function");

    const svelteReactivity = await import(
      pathToFileURL(path.join(distDir, "runtime", "svelte-reactivity.js")).href
    );
    expect(typeof svelteReactivity.SvelteMap).toBe("function");
    expect(typeof svelteReactivity.SvelteSet).toBe("function");
    expect(typeof svelteReactivity.createSubscriber).toBe("function");

    const formula = await import(
      pathToFileURL(path.join(distDir, "runtime", "shadowcat-formula.js")).href
    );
    expect(typeof formula.parseFormula).toBe("function");
    expect(typeof formula.evaluate).toBe("function");
    expect(typeof formula.resolveAll).toBe("function");
    expect(typeof formula.resolveNotationTemplate).toBe("function");
    expect(formula.NOTATION_KEYWORDS).toBeDefined();

    const core = await import(
      pathToFileURL(path.join(distDir, "runtime", "shadowcat-core.js")).href
    );
    expect(typeof core.WsClient).toBe("function");
    expect(typeof core.DocumentStore).toBe("function");

    const uiKit = await import(
      pathToFileURL(path.join(distDir, "runtime", "shadowcat-ui-kit.js")).href
    );
    expect(typeof uiKit.setAppContext).toBe("function");
    expect(typeof uiKit.getAppContext).toBe("function");

    // @shadowcat/types is type-only (fully erased by tsc): a zero-export
    // chunk here is correct, not a regression.
    const types = await import(
      pathToFileURL(path.join(distDir, "runtime", "shadowcat-types.js")).href
    );
    expect(Object.keys(types)).toEqual([]);
  });
});
