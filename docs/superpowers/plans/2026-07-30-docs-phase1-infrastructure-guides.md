# Documentation System Phase 1 — Infrastructure + Guides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution` (per project CLAUDE.md this replaces
> subagent-driven-development / executing-plans on Fable). On any other model use
> superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Build the complete docs pipeline — VitePress portal, TypeDoc + rustdoc
reference generation, assembly/serve/link-check/example-extraction scripts, warn-tier
lint wiring, CI docs job — plus the three tutorial guides, two CI-built worked
examples, per-module portal pages, and the protocol overview.

**Architecture:** A VitePress site at `docs/site/` is the portal; TypeDoc (workspace
`packages` strategy) and `cargo doc --document-private-items` generate the two API
references; `scripts/assemble-docs.mjs` composes everything into git-ignored
`dist-docs/` and link-checks it. Doc comments in source are the durable asset; every
generator is replaceable. Enforcement lands at warn-tier now (informational CI
steps); the sweep phases (separate plans) ratchet to deny.

**Tech Stack:** VitePress, TypeDoc, rustdoc/clippy, eslint-plugin-jsdoc,
svelte-eslint-parser, Node scripts (vitest-tested), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-30-documentation-system-design.md` (this
plan implements the spec's **Phase 1** only; doc-comment sweeps are follow-on plans).

## Model/Effort directives

Plan authored mainline in the requesting session (Fable 5, effort high) at the
user's explicit direction — no model switch, no plan-writer dispatch. Execution:
Fable-class session via `mainline-plan-execution` (inline per-task compliance
checks + ONE dispatched fresh-context final review using the
`shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair, escalating to `-opus`
twins if findings read shallow). No per-task review dispatch.

## Buddy-check directives

No high-risk signals (additive tooling; no security-sensitive code paths, no
destructive migrations, no shared-state concurrency). Standard final review only.

## Global Constraints

- Cross-platform from day one: all scripts use `node:path`/`node:fs` (no hardcoded
  separators, no bash-only syntax); package.json script chains may use `&&` only.
- `dist/` must be built before ANY `cargo` invocation that compiles the server
  (including `cargo doc` — rust-embed validates `../../dist/` at compile time).
- CI clippy runs `-D warnings`: never add warn-tier lint attributes to Rust source
  in this phase; warn-tier lints run as CLI `-W` flags in informational steps only.
- Never delete permanently: use `trash` (never `rm`/`Remove-Item`); scripts that
  clean their own scratch output may overwrite it in place instead.
- No PII/real credentials anywhere; examples use obviously synthetic data.
- Comments: present-tense, no history/process meta, cite algorithm sources,
  document invariants and hidden coupling.
- Commit each task on green (local relevant tests); push only at plan completion.
- New dev-dependencies (user-approved): `vitepress`, `typedoc`,
  `eslint-plugin-jsdoc`, `svelte-eslint-parser`. Root `devDependencies` only —
  install latest with `pnpm add -Dw <pkg>` (lockfile pins).
- Verified API facts used throughout this plan (do not re-derive): `Module` shape
  `{ manifest, register(ctx) }`; `ctx.contributions.contribute({ id, contract,
  component, order?, panel?, sheet? })`; `PANEL_CONTRACT` literal `"shadowcat.panel"`;
  `sheetContract(docType)`; sheet component props `{ docId, systemPrefix, close }`;
  `setField(ctx, docId, path, old, value)` from `@shadowcat/ui-kit`;
  `getAppContext()`; `ctx.documents: { query(docType), get(id), subscribe(cb),
  appliedSeq }`; `getPointer(doc, "/system/...")` JSON-pointer paths;
  `createSubscriber` bridge pattern (ActorSheet.svelte:27); loader accepts
  `{ default: Module }` or bare `Module`; `parseFormula(src): Expr | FormulaError`
  and `evaluate(expr, resolve): FormulaValue` from `@shadowcat/formula`.

---

### Task 1: VitePress portal skeleton

**Files:**
- Modify: `package.json` (root — devDep `vitepress`, scripts)
- Create: `docs/site/.vitepress/config.mts`
- Create: `docs/site/index.md`
- Modify: `.gitignore` (add `dist-docs/`, `.docs-tmp/`, `docs/site/.vitepress/cache/`, `docs/site/.vitepress/dist/`)

**Interfaces:**
- Produces: `pnpm docs:dev` (portal dev server), `pnpm docs:build:portal`
  (VitePress build → `docs/site/.vitepress/dist`). Later tasks add pages under
  `docs/site/guides/`, `docs/site/modules/`, `docs/site/protocol.md` and extend
  `themeConfig.sidebar`/`nav` in `config.mts`.

- [ ] **Step 1: Install VitePress**

Run: `pnpm add -Dw vitepress`

- [ ] **Step 2: Create the VitePress config**

`docs/site/.vitepress/config.mts`:

```ts
import { defineConfig } from "vitepress";

// Portal for the assembled docs site. The generated references (api/ts, api/rust)
// are copied in AFTER this build by scripts/assemble-docs.mjs, so /api/ links are
// outside VitePress's dead-link graph and are validated by the assembly script's
// own link check instead.
export default defineConfig({
  title: "Shadowcat",
  description: "Self-hostable, fully moddable virtual tabletop — documentation",
  srcDir: ".",
  ignoreDeadLinks: [/^\.?\.?\/?api\//],
  themeConfig: {
    nav: [
      { text: "Guides", link: "/guides/hosting" },
      { text: "Modules", link: "/modules/" },
      { text: "Protocol", link: "/protocol" },
      { text: "TS API", link: "/api/ts/", target: "_self" },
      { text: "Rust API", link: "/api/rust/shadowcat/", target: "_self" },
    ],
    sidebar: {
      "/guides/": [
        {
          text: "Guides",
          items: [
            { text: "Hosting a server", link: "/guides/hosting" },
            { text: "Creating a module", link: "/guides/creating-a-module" },
            { text: "Creating a system", link: "/guides/creating-a-system" },
          ],
        },
      ],
      "/modules/": [{ text: "First-party modules", items: [] }],
    },
    search: { provider: "local" },
  },
});
```

Note: the three guide links and the modules index are created by Tasks 9–15; until
then `pnpm docs:build:portal` fails VitePress's dead-link check on the nav/sidebar
entries. That is expected inside this task — Step 3's landing page plus stub-free
sequencing is restored by running the FULL portal build only from Task 4 onward.
For THIS task verify with `pnpm docs:dev` (dev server tolerates missing pages).

- [ ] **Step 3: Create the landing page**

`docs/site/index.md`:

```md
---
layout: home
hero:
  name: Shadowcat
  text: Self-hostable, fully moddable virtual tabletop
  tagline: One native executable. Server-authoritative. Built to be modded.
  actions:
    - theme: brand
      text: Host a server
      link: /guides/hosting
    - theme: alt
      text: Create a module
      link: /guides/creating-a-module
    - theme: alt
      text: Create a system
      link: /guides/creating-a-system
features:
  - title: Guides
    details: Step-by-step tutorials with complete, CI-built example code.
  - title: TypeScript API
    details: Generated reference for every workspace package — @shadowcat/core, ui-kit, formula, types, and all first-party modules.
    link: /api/ts/
  - title: Rust API
    details: Generated reference for the server crate, private items included.
    link: /api/rust/shadowcat/
---

## Reading these docs locally

The portal does not render from `file://` (VitePress emits absolute asset paths).
From a Shadowcat checkout run `pnpm docs:build` once, then `pnpm docs:serve` and
open the printed URL.
```

- [ ] **Step 4: Add root scripts + gitignore entries**

In root `package.json` `scripts` add:

```json
"docs:dev": "vitepress dev docs/site",
"docs:build:portal": "vitepress build docs/site"
```

Append to `.gitignore`: `dist-docs/`, `.docs-tmp/`, `docs/site/.vitepress/cache/`,
`docs/site/.vitepress/dist/`.

- [ ] **Step 5: Verify dev server boots**

Run: `pnpm docs:dev` — expect the landing page at the printed localhost URL, then
stop it. (Full build deferred to Task 4 as noted in Step 2.)

- [ ] **Step 6: Commit**

```bash
git add package.json pnpm-lock.yaml .gitignore docs/site
git commit -m "docs(site): VitePress portal skeleton with landing page"
```

---

### Task 2: TypeDoc reference build

**Files:**
- Modify: `package.json` (root — devDep `typedoc`, script `docs:api:ts`)
- Create: `typedoc.json` (root)
- Create: one `typedoc.json` per workspace package (list in Step 2)

**Interfaces:**
- Produces: `pnpm docs:api:ts` → merged HTML reference at `.docs-tmp/api/ts/`.
  Task 4's assembly copies that folder to `dist-docs/api/ts/`.

- [ ] **Step 1: Install TypeDoc**

Run: `pnpm add -Dw typedoc`

- [ ] **Step 2: Per-package entry configs**

For EVERY workspace package — `src/types`, `src/client/{core,render,ui-kit,shell,formula}`,
and all of `src/modules/*` (currently 20: actors, assets, chat, chat-card,
chat-composer, conditions, core-ui, entry, factions, game-settings, panels,
scene-browser, scene-tools, settings, sheet-actor, sheet-fallback, sheet-item,
stage, statusbar, topbar) — create `<pkg>/typedoc.json`:

```json
{
  "entryPoints": ["src/index.ts"]
}
```

Entry-point resolution rule: use the package's `package.json` `"main"` field; if
absent, inspect the package for its real entry (e.g. the shell is an app — use its
Vite entry, `src/main.ts` or as found). Verify each referenced file exists before
writing the config. If a nested external repo (e.g. `src/modules/nightfox/`) is
present in the checkout, do NOT add a config inside it and exclude it in Step 3 —
it documents itself in its own repo.

- [ ] **Step 3: Root TypeDoc config**

`typedoc.json` (repo root):

```json
{
  "$schema": "https://typedoc.org/schema.json",
  "entryPointStrategy": "packages",
  "entryPoints": ["src/types", "src/client/*", "src/modules/*"],
  "exclude": ["**/node_modules/**", "**/*.test.ts", "**/vitest.setup.ts", "src/modules/nightfox"],
  "out": ".docs-tmp/api/ts",
  "name": "Shadowcat TypeScript API",
  "includeVersion": false,
  "validation": { "notExported": false, "invalidLink": true, "notDocumented": true },
  "requiredToBeDocumented": [
    "Class", "Interface", "Enum", "EnumMember", "Function", "Method",
    "Property", "TypeAlias", "Variable"
  ],
  "treatValidationWarningsAsErrors": false,
  "skipErrorChecking": true
}
```

`treatValidationWarningsAsErrors` stays false until the final ratchet phase.
`skipErrorChecking: true` because packages typecheck via svelte-check (svelte2tsx),
which plain tsc cannot fully reproduce for `.svelte` imports; svelte's published
ambient `*.svelte` module declaration makes the imports resolve as generic
components. If TypeDoc still errors on a `.svelte` import, add an ambient
declaration file and reference it from the affected package's tsconfig — do not
exclude the package.

- [ ] **Step 4: Add script and run**

Add to root scripts: `"docs:api:ts": "typedoc"`.
Run: `pnpm docs:api:ts`
Expected: exit 0 (validation warnings about undocumented symbols are expected and
numerous); `.docs-tmp/api/ts/index.html` exists.

- [ ] **Step 5: Spot-verify a known symbol**

Confirm a page for `setField` (from `@shadowcat/ui-kit`) and one for
`DocumentStore` (from `@shadowcat/core`) exist in the output (search the generated
HTML file names / index). Expected: both render with their existing doc comments.

- [ ] **Step 6: Commit**

```bash
git add package.json pnpm-lock.yaml typedoc.json src/types/typedoc.json src/client/*/typedoc.json src/modules/*/typedoc.json
git commit -m "docs(api): TypeDoc workspace reference build (packages strategy)"
```

---

### Task 3: rustdoc reference build

**Files:**
- Modify: `package.json` (root — script `docs:api:rust`)

**Interfaces:**
- Produces: `pnpm docs:api:rust` → rustdoc HTML at `target/doc/` (crate root page
  `target/doc/shadowcat/index.html`). Task 4 copies `target/doc` →
  `dist-docs/api/rust/` (so the crate page is `api/rust/shadowcat/`).

- [ ] **Step 1: Add the script**

```json
"docs:api:rust": "cargo doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps"
```

Constraint: `cargo doc` compiles the crate, so rust-embed requires `dist/` to
exist. The composed `docs:build` (Task 4) runs `pnpm build` first; when running
`docs:api:rust` standalone, build the client first if `dist/` is absent.

- [ ] **Step 2: Run and verify**

Run: `pnpm build` (if `dist/` absent), then `pnpm docs:api:rust`
Expected: exit 0; `target/doc/shadowcat/index.html` exists; private modules (e.g.
`scene`, `ws`) have pages.

- [ ] **Step 3: Commit**

```bash
git add package.json
git commit -m "docs(api): rustdoc build script (private items included)"
```

---

### Task 4: Assembly, link check, and serve scripts

**Files:**
- Create: `scripts/assemble-docs.mjs`
- Create: `scripts/assemble-docs.test.mjs`
- Create: `scripts/serve-docs.mjs`
- Modify: `package.json` (root — scripts `docs:build`, `docs:serve`, widen `test:scripts`)

**Interfaces:**
- Consumes: `.docs-tmp/api/ts/` (Task 2), `target/doc/` (Task 3),
  `docs/site/.vitepress/dist/` (Task 1).
- Produces: `pnpm docs:build` → assembled `dist-docs/`; `pnpm docs:serve` → local
  static server over `dist-docs/`. Exports for tests: `extractLocalLinks(html)`,
  `checkLinks(rootDir, htmlFiles)`, `assemble(paths)`.

- [ ] **Step 1: Write the failing tests**

`scripts/assemble-docs.test.mjs`:

```js
import { describe, it, expect, beforeEach } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { extractLocalLinks, checkLinks, assemble } from "./assemble-docs.mjs";

describe("extractLocalLinks", () => {
  it("returns local href/src targets, skipping schemes and fragments", () => {
    const html = `<a href="guides/hosting.html">g</a>
      <a href="https://example.com/x">ext</a>
      <a href="#anchor">frag</a>
      <img src="../logo.png">
      <a href="mailto:testuser-01@example.com">m</a>
      <a href="api/ts/index.html#setField">deep</a>`;
    expect(extractLocalLinks(html)).toEqual([
      "guides/hosting.html",
      "../logo.png",
      "api/ts/index.html",
    ]);
  });
});

describe("checkLinks", () => {
  let root;
  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "docs-link-"));
    mkdirSync(join(root, "guides"), { recursive: true });
    writeFileSync(join(root, "index.html"), `<a href="guides/a.html">a</a>`);
    writeFileSync(join(root, "guides", "a.html"), `<a href="../index.html">up</a>`);
  });
  it("passes when every target exists", () => {
    expect(checkLinks(root, [join(root, "index.html"), join(root, "guides", "a.html")])).toEqual([]);
  });
  it("reports missing targets with source file", () => {
    writeFileSync(join(root, "index.html"), `<a href="guides/missing.html">x</a>`);
    const broken = checkLinks(root, [join(root, "index.html")]);
    expect(broken).toHaveLength(1);
    expect(broken[0].target).toContain("missing.html");
  });
  it("treats a directory link as its index.html", () => {
    writeFileSync(join(root, "index.html"), `<a href="guides/">dir</a>`);
    expect(checkLinks(root, [join(root, "index.html")])).toEqual([]);
  });
});

describe("assemble", () => {
  it("copies portal, ts, and rust trees into the output root", () => {
    const src = mkdtempSync(join(tmpdir(), "docs-src-"));
    const out = join(mkdtempSync(join(tmpdir(), "docs-out-")), "dist-docs");
    for (const [dir, file] of [["portal", "index.html"], ["ts", "index.html"], ["rust", "shadowcat.html"]]) {
      mkdirSync(join(src, dir), { recursive: true });
      writeFileSync(join(src, dir, file), "<html></html>");
    }
    assemble({ portal: join(src, "portal"), ts: join(src, "ts"), rust: join(src, "rust"), out });
    for (const p of ["index.html", join("api", "ts", "index.html"), join("api", "rust", "shadowcat.html")]) {
      expect(existsSync(join(out, p))).toBe(true);
    }
  });
});
```

(`existsSync` joins the `node:fs` import at the top of the file.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run scripts/assemble-docs.test.mjs`
Expected: FAIL — module `./assemble-docs.mjs` not found.

- [ ] **Step 3: Implement the script**

`scripts/assemble-docs.mjs`:

```js
// Composes the final dist-docs/ site: VitePress portal at the root, TypeDoc under
// api/ts/, rustdoc under api/rust/, then link-checks the PORTAL pages (generated
// references guarantee their own internal integrity; portal links INTO them are
// validated because the copied files are on disk by check time).
// Cross-platform invariant: node:path/node:fs only — no shell, no separators.
import { cpSync, existsSync, readdirSync, readFileSync, statSync, mkdirSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const SKIP_SCHEMES = /^(?:[a-z][a-z0-9+.-]*:|#)/i;

/** Local link targets (href/src) in one HTML string; fragments/queries stripped. */
export function extractLocalLinks(html) {
  const out = [];
  for (const m of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const raw = m[1];
    if (SKIP_SCHEMES.test(raw)) continue;
    const clean = raw.split("#")[0].split("?")[0];
    if (clean !== "") out.push(decodeURIComponent(clean));
  }
  return out;
}

/** Broken links across htmlFiles, resolved against each file's directory.
 * A trailing-slash or extensionless-directory target resolves to its index.html. */
export function checkLinks(rootDir, htmlFiles) {
  const broken = [];
  for (const file of htmlFiles) {
    const html = readFileSync(file, "utf8");
    for (const link of extractLocalLinks(html)) {
      const base = link.startsWith("/") ? join(rootDir, link) : resolve(dirname(file), link);
      const target = existsSync(base) && statSync(base).isDirectory() ? join(base, "index.html") : base;
      if (!existsSync(target)) broken.push({ source: file, target });
    }
  }
  return broken;
}

/** Recursively list .html files under dir, skipping the given top-level subtrees. */
export function htmlFilesUnder(dir, skipSubtrees = []) {
  const out = [];
  const skip = new Set(skipSubtrees.map((s) => resolve(dir, s)));
  const walk = (d) => {
    if (skip.has(resolve(d))) return;
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(".html")) out.push(p);
    }
  };
  walk(dir);
  return out;
}

/** Copy portal/ts/rust trees into out (portal at root, refs under api/). */
export function assemble({ portal, ts, rust, out }) {
  mkdirSync(out, { recursive: true });
  cpSync(portal, out, { recursive: true });
  cpSync(ts, join(out, "api", "ts"), { recursive: true });
  cpSync(rust, join(out, "api", "rust"), { recursive: true });
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const paths = {
    portal: join(repo, "docs", "site", ".vitepress", "dist"),
    ts: join(repo, ".docs-tmp", "api", "ts"),
    rust: join(repo, "target", "doc"),
    out: join(repo, "dist-docs"),
  };
  for (const [k, p] of Object.entries(paths)) {
    if (k !== "out" && !existsSync(p)) {
      console.error(`assemble-docs: missing input '${k}' at ${p} — run the full pnpm docs:build chain`);
      process.exit(1);
    }
  }
  assemble(paths);
  // Portal pages only; api/ subtrees are excluded as link SOURCES, present as targets.
  const portalPages = htmlFilesUnder(paths.out, [join("api", "ts"), join("api", "rust")]);
  const broken = checkLinks(paths.out, portalPages);
  if (broken.length > 0) {
    for (const b of broken) console.error(`dead link: ${b.source} -> ${b.target}`);
    process.exit(1);
  }
  console.log(`dist-docs assembled: ${portalPages.length} portal pages, links OK (root: ${paths.out}${sep})`);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run scripts/assemble-docs.test.mjs`
Expected: PASS.

- [ ] **Step 5: Serve script**

`scripts/serve-docs.mjs`:

```js
// Minimal static server for dist-docs/ (VitePress output needs a server; its
// absolute asset paths do not render from file://). No dependencies.
import { createServer } from "node:http";
import { createReadStream, existsSync, statSync } from "node:fs";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const ROOT = resolve(fileURLToPath(import.meta.url), "..", "..", "dist-docs");
const PORT = Number(process.argv[2] ?? 4173);
const MIME = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript",
  ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml",
  ".png": "image/png", ".jpg": "image/jpeg", ".gif": "image/gif", ".ico": "image/x-icon",
  ".woff": "font/woff", ".woff2": "font/woff2", ".ttf": "font/ttf", ".map": "application/json",
  ".txt": "text/plain; charset=utf-8", ".wasm": "application/wasm",
};

if (!existsSync(ROOT)) {
  console.error(`dist-docs/ not found — run pnpm docs:build first`);
  process.exit(1);
}

createServer((req, res) => {
  const urlPath = decodeURIComponent((req.url ?? "/").split("?")[0]);
  // INVARIANT: resolved path stays inside ROOT (path-traversal guard).
  let file = normalize(join(ROOT, urlPath));
  if (!file.startsWith(ROOT)) { res.writeHead(403); res.end(); return; }
  if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
  if (!existsSync(file)) { res.writeHead(404); res.end("404"); return; }
  res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
}).listen(PORT, () => console.log(`docs at http://localhost:${PORT}/`));
```

- [ ] **Step 6: Wire the composed build + serve scripts**

Root `package.json` scripts:

```json
"docs:build": "pnpm build && pnpm docs:api:ts && pnpm docs:api:rust && pnpm docs:build:portal && node scripts/assemble-docs.mjs",
"docs:serve": "node scripts/serve-docs.mjs",
"test:scripts": "vitest run scripts/"
```

(`test:scripts` widens from the single check-svelte-runtime-entries file to the
whole `scripts/` directory; verify the existing test still passes.)

- [ ] **Step 7: Full docs build end-to-end**

Run: `pnpm docs:build`
Expected: NOW the VitePress build fails on nav/sidebar dead links (guides pages do
not exist yet — Tasks 9/11/12). Temporarily verify the chain by confirming it
fails at exactly that step with only the known missing pages listed. Then create
the three guide pages as one-line seed pages so the chain completes end-to-end:

`docs/site/guides/hosting.md`, `docs/site/guides/creating-a-module.md`,
`docs/site/guides/creating-a-system.md`, `docs/site/modules/index.md`,
`docs/site/protocol.md` — each seeded with its real H1 title plus one true
sentence of scope (e.g. `# Hosting a Shadowcat server` / "Start-to-finish guide;
full content lands with the docs Phase-1 guide tasks."). These are REAL pages the
guide tasks rewrite in place — not committed placeholders for unplanned work
(their content tasks are Tasks 9–13 of this same plan).

Re-run `pnpm docs:build` → expect exit 0; then `pnpm docs:serve` and spot-check
`/`, `/api/ts/`, `/api/rust/shadowcat/` in a browser.

- [ ] **Step 8: Run script tests + commit**

Run: `pnpm run test:scripts` — expect PASS (both test files).

```bash
git add package.json scripts/assemble-docs.mjs scripts/assemble-docs.test.mjs scripts/serve-docs.mjs docs/site
git commit -m "docs(build): assemble dist-docs with link check + local serve script"
```

---

### Task 5: TS example extraction + typecheck gate

**Files:**
- Create: `scripts/extract-ts-examples.mjs`
- Create: `scripts/extract-ts-examples.test.mjs`
- Create: `scripts/ts-examples-tsconfig.template.json`
- Modify: `package.json` (root — script `docs:check-examples`)

**Interfaces:**
- Produces: `pnpm docs:check-examples` — extracts every `@example` ` ```ts ` block
  from workspace TS sources into `.docs-tmp/examples/` and typechecks them
  (`tsc --noEmit`); exits non-zero listing source file:line on failure. Exports for
  tests: `extractExamples(sourceText)` → `[{ code, line }]`.
  Rule (document in the script header): only ` ```ts ` fences inside `@example`
  tags are typechecked; ` ```svelte ` and untagged fences are ignored by design.

- [ ] **Step 1: Write the failing tests**

`scripts/extract-ts-examples.test.mjs`:

```js
import { describe, it, expect } from "vitest";
import { extractExamples } from "./extract-ts-examples.mjs";

describe("extractExamples", () => {
  it("extracts a ts fence inside an @example tag with its line number", () => {
    const src = [
      "/**",
      " * Adds.",
      " * @example",
      " * ```ts",
      " * const x = add(1, 2);",
      " * ```",
      " */",
      "export function add(a: number, b: number): number { return a + b; }",
    ].join("\n");
    const got = extractExamples(src);
    expect(got).toHaveLength(1);
    expect(got[0].code).toBe("const x = add(1, 2);");
    expect(got[0].line).toBe(4);
  });
  it("ignores svelte fences and fences outside @example", () => {
    const src = [
      "/**",
      " * @example",
      " * ```svelte",
      " * <Foo />",
      " * ```",
      " */",
      "/** ```ts",
      " * notAnExample();",
      " * ``` */",
    ].join("\n");
    expect(extractExamples(src)).toHaveLength(0);
  });
  it("extracts multiple examples from one file", () => {
    const one = "/**\n * @example\n * ```ts\n * a();\n * ```\n */\n";
    expect(extractExamples(one + one)).toHaveLength(2);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run scripts/extract-ts-examples.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

`scripts/ts-examples-tsconfig.template.json`:

```json
{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "skipLibCheck": true,
    "baseUrl": ".",
    "paths": {
      "@shadowcat/core": ["../../src/client/core/src/index.ts"],
      "@shadowcat/ui-kit": ["../../src/client/ui-kit/src/index.ts"],
      "@shadowcat/formula": ["../../src/client/formula/src/index.ts"],
      "@shadowcat/types": ["../../src/types/index.ts"]
    }
  },
  "include": ["*.ts"]
}
```

(Verify `src/types`'s real entry from its `package.json` `main` and correct the
path above if it differs.)

`scripts/extract-ts-examples.mjs`:

```js
// Staleness gate for TS doc examples: every @example ```ts fence in workspace
// sources is extracted to .docs-tmp/examples/ and typechecked (compile-checked,
// not executed — the TS analogue of `no_run` doctests). ```svelte and untagged
// fences are ignored by design.
import { mkdirSync, readdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import process from "node:process";

/** @example ```ts fences in one source text, with the fence's 1-based line. */
export function extractExamples(sourceText) {
  const out = [];
  for (const block of sourceText.matchAll(/\/\*\*[\s\S]*?\*\//g)) {
    const body = block[0];
    if (!/@example/.test(body)) continue;
    const offsetLine = sourceText.slice(0, block.index).split("\n").length;
    for (const fence of body.matchAll(/```ts\n([\s\S]*?)```/g)) {
      const code = fence[1]
        .split("\n")
        .map((l) => l.replace(/^\s*\* ?/, ""))
        .join("\n")
        .trim();
      const fenceLine = offsetLine + body.slice(0, fence.index).split("\n").length - 1;
      if (code !== "") out.push({ code, line: fenceLine });
    }
  }
  return out;
}

/** All candidate .ts files under the given roots (skips node_modules/dist/tests/generated). */
export function candidateFiles(repoRoot, roots) {
  const files = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === "node_modules" || e.name === "dist" || e.name === "generated") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".ts") && !e.name.endsWith(".test.ts")) files.push(p);
    }
  };
  for (const r of roots) {
    const abs = join(repoRoot, r);
    try { walk(abs); } catch { /* root absent (e.g. examples/ before Task 8) */ }
  }
  return files;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const outDir = join(repo, ".docs-tmp", "examples");
  mkdirSync(outDir, { recursive: true });
  const files = candidateFiles(repo, ["src/types", "src/client", "src/modules", "examples"]);
  const index = [];
  let n = 0;
  for (const f of files) {
    for (const ex of extractExamples(readFileSync(f, "utf8"))) {
      const name = `ex${String(n++).padStart(4, "0")}.ts`;
      writeFileSync(join(outDir, name), `// source: ${relative(repo, f)}:${ex.line}\nexport {};\n${ex.code}\n`);
      index.push({ name, source: `${relative(repo, f)}:${ex.line}` });
    }
  }
  copyFileSync(join(repo, "scripts", "ts-examples-tsconfig.template.json"), join(outDir, "tsconfig.json"));
  if (index.length === 0) { console.log("no @example ts blocks found — trivially green"); process.exit(0); }
  const pnpm = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
  const res = spawnSync(pnpm, ["exec", "tsc", "-p", outDir], { stdio: "inherit", shell: process.platform === "win32" });
  if (res.status !== 0) {
    console.error(`example typecheck FAILED — map exNNNN.ts to sources via the header comment in each file (${index.length} examples)`);
    process.exit(res.status ?? 1);
  }
  console.log(`${index.length} TS doc examples typecheck OK`);
}
```

`export {};` per emitted file keeps each example an isolated module (no
cross-example global collisions).

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run scripts/extract-ts-examples.test.mjs`
Expected: PASS.

- [ ] **Step 5: Wire script + run against the real tree**

Add root script: `"docs:check-examples": "node scripts/extract-ts-examples.mjs"`.
Run: `pnpm docs:check-examples`
Expected: exit 0 (few or zero `@example` blocks exist today; if any existing block
fails to typecheck, fix THAT doc comment now — it is a stale example, exactly what
this gate exists to catch).

- [ ] **Step 6: Commit**

```bash
git add package.json scripts/extract-ts-examples.mjs scripts/extract-ts-examples.test.mjs scripts/ts-examples-tsconfig.template.json
git commit -m "docs(build): @example extraction + typecheck staleness gate"
```

---

### Task 6: Warn-tier lint wiring (docs ESLint config)

**Files:**
- Create: `eslint.docs.config.js`
- Modify: `package.json` (root — devDeps `eslint-plugin-jsdoc`,
  `svelte-eslint-parser`; script `lint:docs`)

**Interfaces:**
- Produces: `pnpm lint:docs` — warn-tier jsdoc coverage report over TS + svelte
  script blocks; exits 0 (warnings only) until sweep phases flip severities.
  Sweep plans flip per-package severity to `"error"` in THIS file, and the final
  ratchet phase merges the rules into `eslint.config.js`.

- [ ] **Step 1: Install deps**

Run: `pnpm add -Dw eslint-plugin-jsdoc svelte-eslint-parser`

- [ ] **Step 2: Create the docs lint config**

`eslint.docs.config.js`:

```js
// Doc-coverage lint (spec §2): every function — exported or not — carries a doc
// comment with description, params, and an @example. Warn-tier during the Phase-1
// ratchet; sweep plans flip per-package severity to error here, and the final
// phase merges these rules into eslint.config.js. Kept separate so `pnpm lint`
// stays warning-free until then.
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";
import svelteParser from "svelte-eslint-parser";

const RULES = {
  "jsdoc/require-jsdoc": ["warn", {
    require: {
      FunctionDeclaration: true,
      MethodDefinition: true,
      ClassDeclaration: true,
      ArrowFunctionExpression: false,
      FunctionExpression: false,
    },
    // Arrow/function expressions only when they are named exports or class fields
    // would over-fire on inline callbacks; declarations and methods are the
    // "every function" surface the spec enforces mechanically. Inline callbacks
    // are covered by their enclosing declaration's docs.
  }],
  "jsdoc/require-description": "warn",
  "jsdoc/require-param": "warn",
  "jsdoc/require-param-description": "warn",
  "jsdoc/require-returns": "warn",
  "jsdoc/require-example": ["warn", { exemptNoArguments: false }],
};

export default [
  {
    files: ["src/types/**/*.ts", "src/client/**/*.ts", "src/modules/**/*.ts", "examples/**/*.ts"],
    ignores: [
      "**/node_modules/**", "**/dist/**", "**/*.test.ts", "**/vitest.setup.ts",
      // Generated: doc comments originate in the Rust source types (ts-rs).
      "src/types/generated/**",
    ],
    languageOptions: { parser: tseslint.parser },
    plugins: { jsdoc },
    rules: RULES,
  },
  {
    files: ["src/client/**/*.svelte", "src/modules/**/*.svelte", "examples/**/*.svelte"],
    ignores: ["**/node_modules/**", "**/dist/**"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
    },
    plugins: { jsdoc },
    rules: RULES,
  },
];
```

- [ ] **Step 3: Wire and run**

Add root script: `"lint:docs": "eslint --config eslint.docs.config.js src examples scripts"`
— then run `pnpm lint:docs`.
Expected: exit 0 with a LARGE number of warnings (that is the coverage backlog the
sweep phases burn down). If the svelte block ERRORS (parser/plugin incompatibility
rather than warnings), remove the svelte config block, note the fallback
(review-enforced `.svelte` coverage, per spec §2) in the config's header comment,
and record it in the final task's docs-sync.

- [ ] **Step 4: Confirm main lint is untouched**

Run: `pnpm lint`
Expected: exit 0, zero new warnings (docs rules live only in the separate config).

- [ ] **Step 5: Commit**

```bash
git add package.json pnpm-lock.yaml eslint.docs.config.js
git commit -m "docs(lint): warn-tier jsdoc coverage config (ratchet base)"
```

---

### Task 7: CI docs job + web-job gate

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `pnpm docs:build` (Task 4), `pnpm docs:check-examples` (Task 5),
  `pnpm lint:docs` (Task 6).
- Produces: a `docs` job (blocking: docs build + link check + example gate;
  informational: coverage reports) and a `dist-docs` artifact.

- [ ] **Step 1: Add the example gate to the web job**

In the `web` job, after `- run: pnpm run test:scripts`:

```yaml
      - run: pnpm docs:check-examples
```

- [ ] **Step 2: Add the docs job**

Append to `jobs:` in `.github/workflows/ci.yml`:

```yaml
  # Docs pipeline: portal + generated references assembled and link-checked.
  # Single-OS: the docs site is a build artifact, not platform-gated code; the
  # rust matrix already proves the crate compiles per-OS. The two report steps
  # are warn-tier (exit 0) until the sweep phases ratchet lints to deny.
  docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: pnpm/action-setup@v6
        with:
          version: 9
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - uses: dtolnay/rust-toolchain@stable
      # docs:build starts with the client build (rust-embed needs dist/ before
      # cargo doc), then typedoc, cargo doc, vitepress, assembly + link check.
      - run: pnpm docs:build
      - name: Doc-coverage report (TS, informational)
        run: pnpm lint:docs
      - name: Doc-coverage report (Rust, informational)
        run: cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -W missing-docs -W clippy::missing-docs-in-private-items
      # Example-presence report: rustdoc's missing_doc_code_examples lint is
      # nightly-only; -W keeps it warn-tier (exit 0). continue-on-error covers
      # nightly toolchain breakage per spec §2's degradation clause.
      - uses: dtolnay/rust-toolchain@nightly
      - name: Example-presence report (Rust, nightly, informational)
        continue-on-error: true
        env:
          RUSTDOCFLAGS: "-W rustdoc::missing_doc_code_examples"
        run: cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc
      - name: Upload docs site
        uses: actions/upload-artifact@v4
        with:
          name: dist-docs
          path: dist-docs/
```

- [ ] **Step 3: Validate workflow syntax locally**

Run: `pnpm exec prettier --check .github/workflows/ci.yml` (formatting) and read
the diff once against the existing job style. (Full validation happens on the
milestone push — per project rules, CI monitoring at push time is mandatory.)

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: docs job (build + link check + coverage reports) and example gate"
```

---

### Task 8: Worked example — initiative-tracker module

**Files:**
- Modify: `pnpm-workspace.yaml` (add `examples/*`)
- Create: `examples/module-initiative-tracker/package.json`
- Create: `examples/module-initiative-tracker/module.json`
- Create: `examples/module-initiative-tracker/vite.config.ts`
- Create: `examples/module-initiative-tracker/tsconfig.json`, `svelte.config.js`,
  `vitest.config.ts`, `vitest.setup.ts` (copy from `src/modules/scene-browser/`,
  adjusting relative paths)
- Create: `examples/module-initiative-tracker/src/index.ts`
- Create: `examples/module-initiative-tracker/src/InitiativePanel.svelte`
- Create: `examples/module-initiative-tracker/src/InitiativePanel.test.ts`
- Modify: `.github/workflows/ci.yml` (web job: build examples)

**Interfaces:**
- Consumes: `PANEL_CONTRACT`, `Module`, `getAppContext`, `setField`, `getPointer`,
  `createSubscriber` (verified signatures in Global Constraints).
- Produces: a complete external-layout module the creating-a-module guide (Task 9)
  code-imports region-by-region. Region markers (`// #region <name>` /
  `// #endregion`) around: manifest, register, read-actors, write-initiative.

- [ ] **Step 1: Workspace + scaffolding**

Add `- "examples/*"` to `pnpm-workspace.yaml`. Create the package:

`examples/module-initiative-tracker/package.json`:

```json
{
  "name": "shadowcat-example-initiative-tracker",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/ui-kit": "workspace:*",
    "@shadowcat/types": "workspace:^"
  },
  "devDependencies": {
    "@testing-library/svelte": "^5.3.1",
    "jsdom": "^29.1.1"
  },
  "scripts": {
    "build": "vite build",
    "typecheck": "svelte-check --tsconfig ./tsconfig.json",
    "test": "vitest run"
  }
}
```

`examples/module-initiative-tracker/module.json`:

```json
{
  "id": "example-initiative-tracker",
  "version": "0.1.0",
  "engines": { "shadowcat": "^0.1.0" },
  "dependencies": {},
  "capabilities": [],
  "requirements": [],
  "provides": [],
  "requires": ["shadowcat.panel"]
}
```

`examples/module-initiative-tracker/vite.config.ts` — the external-module build
from `docs/design/module-authoring.md` verbatim (lib entry `src/index.ts`, ES
format, `fileName: () => "index.js"`, externals: `svelte`, `/^svelte\//`,
`@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/formula`, `@shadowcat/types`).

Copy `tsconfig.json`, `svelte.config.js`, `vitest.config.ts`, `vitest.setup.ts`
from `src/modules/scene-browser/`, fixing any relative `extends`/paths for the new
depth. Run `pnpm install` to link the new workspace member. Also create a
`typedoc.json` (`{ "entryPoints": ["src/index.ts"] }`) and extend the root
`typedoc.json` `entryPoints` with `"examples/*"`.

- [ ] **Step 2: Write the failing test**

`src/InitiativePanel.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { rollInitiative, sortEntries, type Entry } from "./index";

describe("rollInitiative", () => {
  it("stays within 1..=20", () => {
    for (let i = 0; i < 200; i++) {
      const r = rollInitiative(() => Math.random());
      expect(r).toBeGreaterThanOrEqual(1);
      expect(r).toBeLessThanOrEqual(20);
    }
  });
  it("is deterministic given a fixed rng", () => {
    expect(rollInitiative(() => 0)).toBe(1);
    expect(rollInitiative(() => 0.999999)).toBe(20);
  });
});

describe("sortEntries", () => {
  it("orders by initiative descending, name ascending on ties", () => {
    const entries: Entry[] = [
      { actorId: "usr_test_001", name: "MOCK_ACTOR_B", initiative: 12 },
      { actorId: "usr_test_002", name: "MOCK_ACTOR_A", initiative: 18 },
      { actorId: "usr_test_003", name: "MOCK_ACTOR_C", initiative: 12 },
    ];
    expect(sortEntries(entries).map((e) => e.name)).toEqual([
      "MOCK_ACTOR_A", "MOCK_ACTOR_B", "MOCK_ACTOR_C",
    ]);
  });
});
```

Run: `pnpm --filter shadowcat-example-initiative-tracker test`
Expected: FAIL — `rollInitiative`/`sortEntries` not exported.

- [ ] **Step 3: Implement the module**

`src/index.ts`:

```ts
// #region manifest
import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import InitiativePanel from "./InitiativePanel.svelte";

/** One tracked combatant row: the actor's doc id, display name, and rolled score. */
export interface Entry {
  actorId: string;
  name: string;
  initiative: number;
}

/**
 * Rolls a d20 initiative score.
 * @example
 * ```ts
 * const score = rollInitiative(() => Math.random()); // 1..=20
 * ```
 */
export function rollInitiative(rng: () => number): number {
  return Math.floor(rng() * 20) + 1;
}

/**
 * Turn order: initiative descending; ties break by name ascending so the order
 * is stable across re-renders.
 * @example
 * ```ts
 * const ordered = sortEntries([{ actorId: "a", name: "MOCK_ACTOR_A", initiative: 3 }]);
 * ```
 */
export function sortEntries(entries: Entry[]): Entry[] {
  return [...entries].sort((a, b) => b.initiative - a.initiative || a.name.localeCompare(b.name));
}

/** Tutorial module: contributes one GM panel that rolls + tracks initiative and
 * writes each roll onto the actor's opaque `system` band. */
const initiativeTracker: Module = {
  manifest: {
    id: "example-initiative-tracker",
    version: "0.1.0",
    dependencies: {},
    requires: [PANEL_CONTRACT],
    provides: [],
    engines: { shadowcat: "^0.1.0" },
  },
  // #endregion manifest
  // #region register
  register(ctx) {
    ctx.contributions.contribute({
      id: "example-initiative-tracker:panel",
      contract: PANEL_CONTRACT,
      component: InitiativePanel,
      // labelKey falls back to its literal value for keys absent from the host
      // catalog — community modules have no i18n registration seam yet.
      panel: { icon: "⚔️", labelKey: "Initiative", gmOnly: true },
    });
  },
  // #endregion register
};

export default initiativeTracker;
```

`src/InitiativePanel.svelte`:

```svelte
<script lang="ts">
  // #region read-actors
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { rollInitiative, sortEntries, type Entry } from "./index";

  const ctx = getAppContext();

  // ctx.documents is a plain-callback store, not a rune: every $derived reading
  // it must subscribe itself or it freezes at first read (see ActorSheet's
  // createSubscriber pattern — same implicit coupling).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actors = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("actor");
  });
  // #endregion read-actors

  let entries = $state<Entry[]>([]);
  let turn = $state(0);
  const current = $derived(entries[turn]);

  // #region write-initiative
  /** Roll for one actor: track it locally and persist the score onto the
   * actor's opaque `system` band (OCC: `old` is the raw current stored value). */
  function roll(actor: WireDocument): void {
    const initiative = rollInitiative(() => Math.random());
    entries = sortEntries([
      ...entries.filter((e) => e.actorId !== actor.id),
      { actorId: actor.id, name: actor.name ?? "Unknown", initiative },
    ]);
    turn = 0;
    const path = "/system/initiative";
    if (ctx.canEdit(actor, path)) {
      setField(ctx, actor.id, path, getPointer(actor, path), initiative);
    }
  }
  // #endregion write-initiative

  /** Advance the turn pointer, wrapping at the end of the round. */
  function next(): void {
    if (entries.length > 0) turn = (turn + 1) % entries.length;
  }
</script>

<div class="initiative">
  <h3>Initiative</h3>
  <ul>
    {#each actors as actor (actor.id)}
      <li>
        <span>{actor.name ?? "Unknown"}</span>
        <button type="button" onclick={() => roll(actor)}>Roll</button>
      </li>
    {/each}
  </ul>
  {#if entries.length > 0}
    <ol>
      {#each entries as e, i (e.actorId)}
        <li class:active={i === turn}>{e.name} — {e.initiative}</li>
      {/each}
    </ol>
    <p>Current: {current?.name}</p>
    <button type="button" onclick={next}>Next turn</button>
  {/if}
</div>

<style>
  .initiative { padding: 0.5rem; }
  /* Touch-sized targets (cross-platform UI invariant). */
  button { min-height: 44px; min-width: 44px; }
  .active { font-weight: bold; }
</style>
```

- [ ] **Step 4: Run tests + typecheck + build**

Run:
- `pnpm --filter shadowcat-example-initiative-tracker test` — expect PASS.
- `pnpm --filter shadowcat-example-initiative-tracker typecheck` — expect clean.
- `pnpm --filter shadowcat-example-initiative-tracker build` — expect
  `examples/module-initiative-tracker/dist/index.js` with `@shadowcat/*`/`svelte`
  left as bare imports (spot-check the emitted file's import lines).

- [ ] **Step 5: CI covers the example build**

In the `web` job (ci.yml), after the shell build step, add:

```yaml
      - name: Build worked examples (guide code must stay buildable)
        run: pnpm --filter "shadowcat-example-*" build
```

(`pnpm -r typecheck/test` pick the example up automatically as a workspace member.)

- [ ] **Step 6: Verify example extraction sees it**

Run: `pnpm docs:check-examples`
Expected: reports ≥2 examples (the two `@example` blocks above) and exits 0.

- [ ] **Step 7: Commit**

```bash
git add pnpm-workspace.yaml pnpm-lock.yaml typedoc.json examples/module-initiative-tracker .github/workflows/ci.yml
git commit -m "docs(examples): initiative-tracker worked module (CI-built)"
```

---

### Task 9: Guide — creating a module

**Files:**
- Rewrite: `docs/site/guides/creating-a-module.md` (seeded in Task 4)
- Rewrite: `docs/design/module-authoring.md` → pointer stub
- Modify: `docs/site/.vitepress/config.mts` (only if sidebar text needs adjusting)

**Interfaces:**
- Consumes: `examples/module-initiative-tracker/` region markers (Task 8).
- Produces: the complete module-authoring tutorial; `docs/design/module-authoring.md`
  reduced to a pointer.

- [ ] **Step 1: Write the guide**

`docs/site/guides/creating-a-module.md` — full tutorial with these sections, in
order. Code is NEVER pasted: use VitePress code-import
(`<<< @/../../examples/module-initiative-tracker/src/index.ts#manifest` — verify
the `@` alias root against VitePress srcDir and adjust the relative prefix so
imports resolve; the build's dead-link/snippet check fails loudly if wrong):

1. **What a module is** — client-only contribution package; server runs no
   third-party code; admin-trusted (no sandbox — state this plainly, from
   module-authoring.md "Known limits").
2. **Scaffold** — copy `examples/module-initiative-tracker/` as the starting
   layout; file-by-file inventory table.
3. **The manifest** — `module.json` field-by-field (id, version, engines gate
   semantics at enable AND load time, folder-name-is-identity rule, `entry`
   override, dependencies, provides/requires, requirements-are-advisory).
   Import the manifest region.
4. **The build** — Vite lib mode + externals; the exact runtime import-map set
   (the 8 resolvable specifiers from module-authoring.md) and the two failure
   modes (unserved `svelte/*` subpath, package-root-only resolution) as a
   warning box.
5. **Registering a contribution** — `Module` shape, `register(ctx)`,
   `PANEL_CONTRACT`, `PanelMeta` (icon/labelKey/gmOnly/defaultPlacement; labelKey
   raw-string fallback for community modules). Import the register region.
6. **Reading documents** — `ctx.documents` (query/get/subscribe), the
   createSubscriber freeze gotcha. Import the read-actors region.
7. **Writing documents** — optimistic intents, `setField` + OCC `old` pre-image,
   `ctx.canEdit` advisory gate, server remains authoritative. Import the
   write-initiative region.
8. **Install + enable + dev loop** — build → copy `dist/index.js` + `module.json`
   into `<data-dir>/modules/<folder-id>/`; GM Settings → Installed modules →
   enable per world; reload. Dev flow: nest the module repo in
   `src/modules/<id>/` per module-authoring.md steps 1–3 (reproduce them here —
   this guide REPLACES that doc).
9. **Testing** — unit tests via vitest in the module package; e2e via the
   `test_server` route (summarize module-authoring.md "Testing" section).
10. **Reference** — links into `/api/ts/` for every symbol used.

- [ ] **Step 2: Reduce module-authoring.md to a pointer**

Replace the body of `docs/design/module-authoring.md` with:

```md
# Authoring an External Shadowcat Module

Superseded: this guide moved into the documentation site —
`docs/site/guides/creating-a-module.md` (build with `pnpm docs:build`, view with
`pnpm docs:serve`). The site version is the maintained one; it code-imports the
CI-built `examples/module-initiative-tracker/` so its samples cannot rot.
```

Grep for references to `docs/design/module-authoring.md` across the repo
(`.claude/skills/`, `docs/`, `src/`) and update each to point at the new guide
(the skill-file updates themselves belong to Task 16's gate).

- [ ] **Step 3: Build + verify**

Run: `pnpm docs:build`
Expected: exit 0; snippet imports resolved (VitePress errors on a missing snippet
file); link check green. Open via `pnpm docs:serve` and read the page once for
rendering sanity (code regions present, no raw `<<<` lines).

- [ ] **Step 4: Commit**

```bash
git add docs/site docs/design/module-authoring.md
git commit -m "docs(guides): creating-a-module tutorial; absorb module-authoring.md"
```

---

### Task 10: Worked example — minimal system

**Files:**
- Create: `examples/system-minimal/` — same scaffolding set as Task 8
  (package.json name `shadowcat-example-system-minimal`, module.json id
  `example-system-minimal`, vite.config.ts, tsconfig, svelte/vitest configs,
  typedoc.json)
- Create: `examples/system-minimal/src/index.ts`
- Create: `examples/system-minimal/src/rules.ts`
- Create: `examples/system-minimal/src/CharacterSheet.svelte`
- Create: `examples/system-minimal/src/rules.test.ts`

**Interfaces:**
- Consumes: `sheetContract("actor")` + sheet meta `{ priority }` (higher priority
  wins over sheet-actor's 0); sheet component props `{ docId, systemPrefix, close }`;
  `parseFormula`/`evaluate` from `@shadowcat/formula`.
- Produces: region markers for Task 11: manifest, sheet-registration, rules,
  sheet-read, sheet-write.

- [ ] **Step 1: Scaffold**

Mirror Task 8 Step 1 for `examples/system-minimal/` (workspace already includes
`examples/*`). module.json:

```json
{
  "id": "example-system-minimal",
  "version": "0.1.0",
  "engines": { "shadowcat": "^0.1.0" },
  "dependencies": {},
  "capabilities": [],
  "requirements": [],
  "provides": [{ "contract": "shadowcat.sheet:actor", "cardinality": "multi" }],
  "requires": []
}
```

Add `@shadowcat/formula: workspace:*` to its dependencies (alongside core/ui-kit/types).

- [ ] **Step 2: Write the failing rules test**

`src/rules.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { abilityMod, derived } from "./rules";

describe("abilityMod", () => {
  it("computes the d20-family modifier", () => {
    expect(abilityMod(10)).toBe(0);
    expect(abilityMod(8)).toBe(-1);
    expect(abilityMod(18)).toBe(4);
  });
});

describe("derived", () => {
  it("evaluates formulas against the system body", () => {
    const sys = { attributes: { str: 16, dex: 12 } };
    expect(derived("@attributes.str + @attributes.dex", sys)).toBe(28);
  });
  it("returns null on a malformed formula instead of throwing", () => {
    expect(derived("@attributes.str +", { attributes: { str: 1 } })).toBeNull();
  });
});
```

Run: `pnpm --filter shadowcat-example-system-minimal test` — expect FAIL.

- [ ] **Step 3: Implement rules.ts**

```ts
// #region rules
import { parseFormula, evaluate, type FormulaValue } from "@shadowcat/formula";

/** d20-family ability modifier: floor((score - 10) / 2). */
export function abilityMod(score: number): number {
  return Math.floor((score - 10) / 2);
}

/**
 * Evaluates a formula string against an actor's opaque `system` body, resolving
 * `@a.b.c` references as paths into it. Returns null (never throws) on parse or
 * evaluation failure — degenerate sheet data must not crash the sheet.
 * @example
 * ```ts
 * const total = derived("@attributes.str + 2", { attributes: { str: 16 } }); // 18
 * ```
 */
export function derived(formula: string, system: unknown): number | null {
  const expr = parseFormula(formula);
  if ("error" in (expr as object)) return null;
  try {
    const value: FormulaValue = evaluate(expr as Parameters<typeof evaluate>[0], (path) => {
      let node: unknown = system;
      for (const key of path) node = (node as Record<string, unknown> | undefined)?.[key];
      return node as FormulaValue;
    });
    return typeof value === "number" ? value : null;
  } catch {
    return null;
  }
}
// #endregion rules
```

Verification note for the implementer: check `FormulaError`'s real discriminant in
`src/client/formula/src/types.ts` (`"error" in` vs a kind tag) and `FormulaValue`'s
members, and adjust the guard to the actual shapes — the test suite pins behavior.

- [ ] **Step 4: Run rules tests**

Run: `pnpm --filter shadowcat-example-system-minimal test` — expect PASS.

- [ ] **Step 5: Sheet + registration**

`src/index.ts`:

```ts
// #region manifest
import { sheetContract, type Module } from "@shadowcat/core";
import CharacterSheet from "./CharacterSheet.svelte";

/** Tutorial system: replaces the generic actor sheet with a minimal d20-style
 * character sheet (attributes + formula-derived values on the `system` band). */
const systemMinimal: Module = {
  manifest: {
    id: "example-system-minimal",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract("actor"), cardinality: "multi" }],
    engines: { shadowcat: "^0.1.0" },
  },
  // #endregion manifest
  // #region sheet-registration
  register(ctx) {
    ctx.contributions.contribute({
      id: "example-system-minimal:actor-sheet",
      contract: sheetContract("actor"),
      component: CharacterSheet,
      // Priority 1 outranks the built-in generic actor sheet (priority 0):
      // a game system claims the doc_type by registering higher.
      sheet: { priority: 1 },
    });
  },
  // #endregion sheet-registration
};

export default systemMinimal;
```

`src/CharacterSheet.svelte` — mirror ActorSheet.svelte's prop/reactivity contract
exactly (props `{ docId, systemPrefix, close }`; `createSubscriber` bridge; doc via
`ctx.documents.get(docId)`; reads via `getPointer(doc, `${systemPrefix}/...`)`).
Body:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { abilityMod, derived } from "./rules";

  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx = getAppContext();
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // #region sheet-read
  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const ATTRS = ["str", "dex", "con"] as const;
  /** Current attribute score from the opaque system band (default 10). */
  function score(attr: string): number {
    const v = doc ? getPointer(doc, `${systemPrefix}/attributes/${attr}`) : undefined;
    return typeof v === "number" ? v : 10;
  }
  const power = $derived.by((): number | null => {
    subscribe();
    return doc ? derived("@attributes.str + @attributes.con", getPointer(doc, systemPrefix)) : null;
  });
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));
  // #endregion sheet-read

  // #region sheet-write
  /** Writes one attribute with its OCC pre-image (raw current stored value). */
  function setScore(attr: string, value: number): void {
    if (!doc) return;
    const path = `${systemPrefix}/attributes/${attr}`;
    setField(ctx, docId, path, getPointer(doc, path), value);
  }
  // #endregion sheet-write
</script>

<div class="sheet" role="dialog" aria-label="Character sheet">
  <header>
    <h2>{doc?.name ?? "Character"}</h2>
    <button type="button" aria-label="Close" onclick={close}>×</button>
  </header>
  {#if doc}
    {#each ATTRS as attr (attr)}
      <label>
        {attr.toUpperCase()}
        <input
          type="number"
          value={score(attr)}
          disabled={readOnly}
          onchange={(e) => setScore(attr, Number(e.currentTarget.value))}
        />
        <span>mod {abilityMod(score(attr))}</span>
      </label>
    {/each}
    <p>Power (str + con): {power ?? "—"}</p>
  {/if}
</div>

<style>
  .sheet { padding: 0.5rem; }
  input { min-height: 44px; }
  button { min-height: 44px; min-width: 44px; }
</style>
```

- [ ] **Step 6: Full package verification**

Run: `pnpm --filter shadowcat-example-system-minimal test`, `... typecheck`,
`... build`, then `pnpm docs:check-examples`.
Expected: all green; the `derived` example counted by the extractor.

- [ ] **Step 7: Commit**

```bash
git add examples/system-minimal pnpm-lock.yaml
git commit -m "docs(examples): system-minimal worked system (sheet + formula)"
```

---

### Task 11: Guide — creating a system

**Files:**
- Rewrite: `docs/site/guides/creating-a-system.md` (seeded in Task 4)

**Interfaces:**
- Consumes: `examples/system-minimal/` region markers (Task 10).

- [ ] **Step 1: Write the guide**

Sections, code-importing the Task 10 regions:

1. **Systems are modules** — same toolchain/manifest as the module guide (link
   it; don't repeat); what makes it a *system*: claiming doc types via sheets,
   rules on the opaque `system` band, templates for content.
2. **The three-band document** — envelope `name` / typed `engine` (server-validated,
   17 engine doc types) / opaque `system` (structurally-validated only; the
   server NEVER semantically validates it — cite ARCHITECTURE §2 invariant 6).
   This is the core mental model for system authors.
3. **Claiming the actor sheet** — `sheetContract("actor")`, priority contest vs
   the generic sheet (import sheet-registration region); the sheet prop contract
   (`docId`/`systemPrefix`/`close`) and WHY systemPrefix exists (top-level vs
   instanced-token embedded actors).
4. **Reading and writing the system band** — getPointer JSON-pointer paths, OCC
   `old` pre-image discipline, canEdit (import sheet-read + sheet-write regions).
5. **Rules via @shadowcat/formula** — parseFormula/evaluate, fail-closed on
   malformed input (import rules region).
6. **Templates** — stamp/pull/push/revert (`ctx.templates`), `source`-based
   provenance, "any document can be a template"; brief, linking `/api/ts/` for
   the TemplatesApi surface.
7. **Dice + chat integration** — pointer-level: rolls go through chat commands,
   roll immutability, link to the protocol page + `/api/ts/` chat types.
8. **The full-scale reference** — Nightfox: framework-neutral formula library +
   rules engine + sheets layer; where each concern lives in its repo.

- [ ] **Step 2: Build + verify + commit**

Run: `pnpm docs:build` — expect green (snippets resolve, links check).

```bash
git add docs/site
git commit -m "docs(guides): creating-a-system tutorial"
```

---

### Task 12: Guide — hosting a server

**Files:**
- Rewrite: `docs/site/guides/hosting.md` (seeded in Task 4)

- [ ] **Step 1: Verify the flag/env/first-run facts**

Before writing, confirm against source (values below were read from
`src/server/src/config.rs` and `src/server/src/main.rs` at plan time; re-verify):
the full `Cli` flag set (`--bind`, `--db`, `--config`, `--admin-user`,
`--admin-password`, `--setup-token`, `--session-key`, `--assets-dir`,
`--modules-dir`, `--backups-dir`, `--backup-to`, `--restore-from`, `--force`),
the figment layering order (CLI flag > `SHADOWCAT_*` env > TOML > default), the
default bind/db values, the setup-token first-run flow and admin-created-accounts
+ world invite/accept seating (from `src/server/src/auth/` + `http/`), and the
upload/login/invite rate-limit config keys (config.rs lines 66–87).

- [ ] **Step 2: Write the guide**

Sections:

1. **Get the binary** — download a release artifact (macOS .app / Linux staging
   tree / Windows .exe per CI packaging) or build from source: pnpm install →
   `pnpm build` → `cargo build --release --manifest-path src/server/Cargo.toml`
   (state the dist-before-cargo invariant and why).
2. **First run** — start `shadowcat`, what it prints, first-admin bootstrap
   (`--admin-user`/`--admin-password` vs setup token), logging in.
3. **Configuration** — the layering table (CLI > `SHADOWCAT_*` env > TOML via
   `--config` > default) with one worked example of the same key set three ways;
   full config-key reference table (every `Config` field, its flag, its env var,
   its default, one-line meaning).
4. **Worlds and players** — create a world, admin-created accounts, invite/accept
   seating, roles (GM vs player; server admin vs user orthogonality).
5. **Data on disk** — db file, assets dir, modules dir, backups dir; all paths
   OS-portable; where each OS puts them by default.
6. **Backup and restore** — one-shot `shadowcat --backup-to <dir>` /
   `--restore-from <dir> [--force]` (mutually exclusive; VACUUM-INTO snapshot +
   assets; what the printed manifest line means).
7. **Serving to your table** — LAN (bind address + firewall note per OS),
   reverse-proxy (WebSocket upgrade must be forwarded; one nginx location block
   example), HTTPS note (terminate at the proxy).
8. **Mobile** — players join from phone browsers; responsive client, no install.
9. **Troubleshooting** — port in use, db locked (single-connection pool), module
   not appearing (engines gate, folder-name identity), backup refuses non-empty
   dir without `--force`.

- [ ] **Step 3: Build + verify + commit**

Run: `pnpm docs:build` — green.

```bash
git add docs/site
git commit -m "docs(guides): hosting-a-server guide"
```

---

### Task 13: Protocol overview page

**Files:**
- Rewrite: `docs/site/protocol.md` (seeded in Task 4)

- [ ] **Step 1: Write the page**

Source of truth: the discriminated unions in `src/client/core/src/wire.ts`
(`ServerMsgSchema`, and the client→server message schema in the same file) plus
the scene-channel frames (`SceneFrame`, `MoveStream`, `PathResult`). Enumerate
EVERY variant — the page fails its purpose if one is missing; cross-check the
variant list against the file at authoring time. Structure:

1. **Connection lifecycle** — HTTP login (session cookie) → WebSocket → `welcome`
   (world, current_seq, server_version, grants, contract/schema declarations) →
   event stream.
2. **Sequencing + resync** — seq numbers, `resync_begin`/... variants, the
   optimistic client's appliedSeq watermark.
3. **Intents and events** — client ops (create/delete/update + FieldChange
   semantics incl. `remove`), correlated `event`/`reject`, per-recipient
   filtering (hidden fields stripped BEFORE transmission — cite ARCHITECTURE §2
   invariant 4).
4. **Frame catalog** — one table per direction: variant name, purpose (one line),
   link to its generated type under `/api/ts/` (ts-rs types carry the Rust doc
   comments).
5. **Scene channels** — SceneDerived subscription model, MoveStream/vision
   streaming, per-recipient clipping (one paragraph each, links to types).

- [ ] **Step 2: Build + verify + commit**

Run: `pnpm docs:build` — green; every `/api/ts/` link in the catalog resolves
(the link check validates this mechanically).

```bash
git add docs/site
git commit -m "docs(site): wire-protocol overview page"
```

---

### Task 14: Per-module portal pages — shell + infrastructure modules

**Files:**
- Rewrite: `docs/site/modules/index.md` (seeded in Task 4)
- Create: `docs/site/modules/<id>.md` for: `entry`, `core-ui`, `topbar`,
  `statusbar`, `settings`, `game-settings`, `panels`, `assets`, `scene-browser`,
  `sheet-fallback`
- Modify: `docs/site/.vitepress/config.mts` (sidebar `/modules/` items)

**Interfaces:**
- Produces: the fixed per-module page shape Task 15 repeats: **Purpose** (2–4
  sentences) / **Contributions** (table: id, contract, meta — from the module's
  `src/index.ts` manifest, verified against source at authoring time) /
  **Components** (each `.svelte` file, one line each — this is the Svelte-gap
  mitigation, spec §2) / **Contracts & seams** (provides/requires/AppContext
  seams touched) / **Pointers** (source dir link + `/api/ts/` package link).

- [ ] **Step 1: Write the modules index**

`docs/site/modules/index.md`: what first-party modules are (UI-as-modules,
seam-only communication — never importing each other), the full module table
(name → one-line purpose → page link), and a note that external/community modules
follow the creating-a-module guide. If a nested `src/modules/nightfox/` checkout
exists, note Nightfox documents itself in its own repo.

- [ ] **Step 2: Write the 10 pages**

For each module: read its `src/index.ts` (manifest + contribute calls) and its
`.svelte` files' header comments; fill the fixed shape. No speculation — every
claim traceable to source read during authoring.

- [ ] **Step 3: Sidebar + build + commit**

Add the 10 pages + index to the `/modules/` sidebar. Run `pnpm docs:build` — green.

```bash
git add docs/site
git commit -m "docs(modules): portal pages for shell + infrastructure modules"
```

---

### Task 15: Per-module portal pages — gameplay modules

**Files:**
- Create: `docs/site/modules/<id>.md` for: `stage`, `scene-tools`, `actors`,
  `factions`, `conditions`, `chat`, `chat-composer`, `chat-card`, `sheet-actor`,
  `sheet-item`
- Modify: `docs/site/.vitepress/config.mts` (sidebar)

**Interfaces:**
- Consumes: the fixed page shape from Task 14 (repeat it exactly).

- [ ] **Step 1: Write the 10 pages** (same method as Task 14 Step 2)

- [ ] **Step 2: Sidebar + build + commit**

Run `pnpm docs:build` — green.

```bash
git add docs/site
git commit -m "docs(modules): portal pages for gameplay modules"
```

---

### Task 16: Docs-sync, skill gate, and full verification

**Files:**
- Modify: `docs/PLAN.md` (documentation-campaign milestone entry: Phase 1 complete,
  sweep phases listed as upcoming)
- Modify: `docs/TODO.md` (log: community modules have no i18n registration seam —
  labelKey falls back to the raw string; prerequisite verified absent at plan time)
- Modify: `.claude/skills/shadowcat-codebase-core/SKILL.md` (Pointers → build
  commands: add `pnpm docs:build` / `docs:serve` / `docs:check-examples` /
  `lint:docs`; knowledge-layer map: add the docs site as the user-facing layer)
- Modify: `.claude/skills/shadowcat-codebase-module-toolchain/SKILL.md` (pointer:
  module-authoring.md → `docs/site/guides/creating-a-module.md`; note the two
  in-repo `examples/` packages as copyable scaffolds)

- [ ] **Step 1: Update PLAN.md + TODO.md** (content per Files list; verify the
  i18n-seam gap is still real by grepping ui-kit for a module-facing locale
  registration API before logging it)

- [ ] **Step 2: Update the two skills** (content per Files list)

- [ ] **Step 3: Skill-update review gate**

Dispatch `shadowcat-spec-reviewer` (effort: high) on the two skill diffs plus the
module-authoring.md pointer change: confirm no omission/drift/broken pointer.
Brief must state the delivery channel ("report findings in your final message")
and the no-destructive-git rule. Fix findings, re-verify.

- [ ] **Step 4: Full local verification sweep**

Run, all green required:
- `pnpm -r typecheck && pnpm -r test && pnpm lint && pnpm run test:scripts`
- `pnpm docs:check-examples`
- `pnpm --filter "shadowcat-example-*" build`
- `pnpm docs:build` (end-to-end incl. link check)
- `cargo test --manifest-path src/server/Cargo.toml` (doctest baseline unchanged)
- `cargo fmt --all --check` + `cargo clippy --all-targets -- -D warnings` (from
  `src/server/` in a subshell — cwd discipline)

- [ ] **Step 5: Commit + push + monitor CI**

```bash
git add docs/PLAN.md docs/TODO.md .claude/skills
git commit -m "docs(sync): Phase-1 docs campaign state + skill pointers"
git push origin main
gh run watch
```

CI must go green on all five jobs (rust ×3 OS, web, e2e, ui-e2e, docs). Fix-forward
on any red, topmost error first.

---

## Deferred (logged, not dropped)

- Doc-comment sweeps (spec Phases 2–N): one plan per subsystem; each flips its
  area's lints to deny. NOT part of this plan.
- Final ratchet phase (repo-wide deny + `treatValidationWarningsAsErrors: true` +
  merge docs lint into main config): after the last sweep.
- Community-module i18n registration seam: logged to TODO.md in Task 16.
