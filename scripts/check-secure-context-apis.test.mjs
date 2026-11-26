import { test, expect } from "vitest";
import { existsSync, renameSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";
import { scanSecureContext, scanSecureContextDetailed, isCovered, assertGateLive, runGate, allowedFileSet, svelteRegions, SEAM_FILE, HTTPS_SPEC_FILE, MIC_SEAM_FILE, RULES } from "./check-secure-context-apis.mjs";

test("a raw crypto.randomUUID call is a violation", () => {
  const v = scanSecureContext("src/modules/chat/src/ChatPanel.svelte", "const a = 1;\nconst id = crypto.randomUUID();\n");
  expect(v.map((x) => [x.line, x.rule.id])).toEqual([[2, "randomUUID"]]);
});

test("aliased and destructured spellings are violations", () => {
  expect(scanSecureContext("src/a.ts", "const { randomUUID } = crypto;\n")).toHaveLength(1);
  expect(scanSecureContext("src/a.ts", "globalThis.crypto.randomUUID()\n")).toHaveLength(1);
  expect(scanSecureContext("src/a.ts", "self.crypto['randomUUID']()\n")).toHaveLength(1);
});

test("the helper file and its test may name it; every other file may not", () => {
  expect(scanSecureContext("src/client/core/src/uuid.ts", "crypto.randomUUID()")).toEqual([]);
  expect(scanSecureContext("src\\client\\core\\src\\uuid.test.ts", "crypto.randomUUID()")).toEqual([]);
  expect(scanSecureContext("src/client/shell/e2e/fixtures.ts", "delete Crypto.prototype.randomUUID")).toHaveLength(1);
  expect(scanSecureContext("src/client/core/src/uuid2.ts", "crypto.randomUUID()")).toHaveLength(1);
});

test("the HTTPS e2e spec may name the UUID member and no other gated member", () => {
  expect(scanSecureContext(HTTPS_SPEC_FILE, "typeof crypto.randomUUID")).toEqual([]);
  expect(scanSecureContext(HTTPS_SPEC_FILE, "crypto.subtle")).toHaveLength(1);
  expect(scanSecureContext(HTTPS_SPEC_FILE, "navigator.mediaDevices")).toHaveLength(1);
  expect(scanSecureContext("src/client/shell/e2e/https-same-port.spec.ts", "crypto.randomUUID()")).toHaveLength(1);
  expect(RULES.find((r) => r.id === "randomUUID").allowedFiles).toHaveLength(3);
});

test("the replacement helper's own name is not a violation", () => {
  expect(scanSecureContext("src/a.ts", "const id = randomUuid();\n")).toEqual([]);
});

test("coverage: ts/svelte/js/mjs under src and examples, not generated or server", () => {
  expect(isCovered("src/modules/chat/src/ChatPanel.svelte")).toBe(true);
  expect(isCovered("src/client/shell/e2e/levels.spec.ts")).toBe(true);
  expect(isCovered("examples/x/src/index.ts")).toBe(true);
  expect(isCovered("src/types/generated/A.ts")).toBe(false);
  expect(isCovered("src/server/src/lib.rs")).toBe(false);
  expect(isCovered("scripts/x.mjs")).toBe(false);
});

test("every rule names its replacement", () => {
  for (const r of RULES) expect(r.replacement.length).toBeGreaterThan(0);
});

const UNUSED_MEMBERS = [
  ["crypto.subtle", "const d = await crypto.subtle.digest('SHA-256', buf);"],
  ["crypto.subtle", "window.crypto.subtle.importKey()"],
  ["navigator.clipboard", "await navigator.clipboard.writeText(s);"],
  ["navigator.serviceWorker", "navigator.serviceWorker.register('/sw.js');"],
  ["navigator.storage", "await navigator.storage.persist();"],
  ["navigator.locks", "navigator.locks.request('k', () => {});"],
  ["navigator.credentials", "navigator.credentials.get({});"],
  ["navigator.share", "navigator.share({ title: 't' });"],
  ["navigator.share", "if (navigator.canShare(d)) {}"],
  ["navigator.gpu", "const a = await navigator.gpu.requestAdapter();"],
];

test.each(UNUSED_MEMBERS)("a use of %s is a violation (positive control): %s", (id, code) => {
  const v = scanSecureContext("src/modules/x/src/a.ts", code + "\n");
  expect(v.map((x) => x.rule.id)).toEqual([id]);
});

test("the banned members are matched in Svelte files and in comments", () => {
  expect(scanSecureContext("src/modules/x/src/A.svelte", "<script>\n  navigator.clipboard.writeText(a);\n</script>")).toHaveLength(1);
  expect(scanSecureContext("src/a.ts", "// call navigator.locks.request here")).toHaveLength(1);
});

test("unrelated members and prose do not match", () => {
  expect(scanSecureContext("src/a.ts", "navigator.hardwareConcurrency; const subtle = 1; const mediaDevices = 2;")).toEqual([]);
});

test("assertGateLive throws on an empty scan, a scan that misses the seam file, and a scan with no .svelte file", () => {
  expect(() => assertGateLive([])).toThrow(/inert/);
  expect(() => assertGateLive(["src/a.ts"])).toThrow(/inert/);
  expect(() => assertGateLive(["src/client/core/src/uuid.ts", "src/a.ts"])).toThrow(/svelte/);
  expect(() => assertGateLive(["src\\client\\core\\src\\uuid.ts", "src/A.svelte"])).not.toThrow();
  expect(SEAM_FILE).toBe("src/client/core/src/uuid.ts");
});

test("only the randomUUID and microphone rules have allowed files, and those are exactly the seams", () => {
  expect(RULES.filter((r) => r.allowedFiles.length > 0).map((r) => r.id)).toEqual(["randomUUID", "navigator.mediaDevices", "AudioContext.audioWorklet"]);
  // A member the client only READS as a property yields undefined on an insecure origin and never
  // throws, so it has no rule.
  expect(RULES.some((r) => r.members.includes("deviceMemory"))).toBe(false);
  expect(RULES.find((r) => r.id === "navigator.mediaDevices").allowedFiles).toEqual([MIC_SEAM_FILE]);
  expect(RULES.find((r) => r.id === "AudioContext.audioWorklet").allowedFiles).toEqual([MIC_SEAM_FILE]);
  expect(MIC_SEAM_FILE).toBe("src/modules/ducking/src/micVad.ts");
});

test.each([
  ["navigator.mediaDevices", "navigator.mediaDevices.getUserMedia({ audio: true })"],
  ["AudioContext.audioWorklet", "await ctx.audioWorklet.addModule(url);"],
])("a planted access to %s outside the microphone seam fails (positive control): %s", (id, code) => {
  const v = scanSecureContext("src/modules/audio/src/other.ts", code + "\n");
  expect(v.map((x) => x.rule.id)).toEqual([id]);
});

test("the microphone seam file may name both members", () => {
  expect(scanSecureContext(MIC_SEAM_FILE, "navigator.mediaDevices?.getUserMedia; ctx.audioWorklet?.addModule")).toEqual([]);
  expect(scanSecureContext("src\\modules\\ducking\\src\\micVad.ts", "navigator.mediaDevices")).toEqual([]);
});

test("the seam's neighbour files are not the seam", () => {
  expect(scanSecureContext("src/modules/ducking/src/micVad.test.ts", "navigator.mediaDevices")).toHaveLength(1);
  expect(scanSecureContext("src/modules/ducking/src/vad.worklet.ts", "audioContext.audioWorklet.addModule")).toHaveLength(1);
});

// ---- Structural spellings: each must be caught by the parser pass even though no dotted
// `owner.member` text appears. Every row is a positive control.
const STRUCTURAL = [
  ["navigator.mediaDevices", "const { mediaDevices } = navigator;"],
  ["navigator.mediaDevices", "const { mediaDevices: md } = window.navigator;"],
  ["navigator.mediaDevices", "const { navigator: { mediaDevices } } = window;"],
  ["navigator.mediaDevices", "let m; ({ mediaDevices: m } = navigator);"],
  ["navigator.mediaDevices", "navigator?.mediaDevices?.getUserMedia();"],
  ["navigator.mediaDevices", "navigator['mediaDevices'].getUserMedia();"],
  ["navigator.mediaDevices", "navigator?.['mediaDevices'];"],
  ["navigator.mediaDevices", "navigator\n  .mediaDevices\n  .getUserMedia();"],
  ["navigator.mediaDevices", "(navigator as unknown as Navigator).mediaDevices;"],
  ["navigator.mediaDevices", "globalThis.navigator.mediaDevices;"],
  ["navigator.mediaDevices", "globalThis['navigator']['mediaDevices'];"],
  ["navigator.clipboard", "navigator['clipboard'].writeText(s);"],
  ["navigator.clipboard", "const { clipboard } = navigator;"],
  ["navigator.share", "const { share, canShare } = navigator;"],
  ["AudioContext.audioWorklet", "const { audioWorklet } = ctx;"],
  ["AudioContext.audioWorklet", "ctx?.audioWorklet?.addModule(u);"],
  ["AudioContext.audioWorklet", "ctx['audioWorklet'].addModule(u);"],
  ["AudioContext.audioWorklet", "ctx\n  .audioWorklet\n  .addModule(u);"],
  ["crypto.subtle", "const { subtle } = crypto;"],
  ["crypto.subtle", "crypto['subtle'].digest('SHA-256', b);"],
  ["crypto.subtle", "self.crypto?.subtle;"],
  ["navigator.mediaDevices", "({ navigator: { mediaDevices: m } } = window);"],
  ["navigator.mediaDevices", "let m; ({ mediaDevices: m } = globalThis.navigator);"],
  ["AudioContext.audioWorklet", "let w; ({ a: { audioWorklet: w } } = anything);"],
  ["AudioContext.audioWorklet", "const { a: { audioWorklet } } = x;"],
  ["AudioContext.audioWorklet", "const { a: { b: { audioWorklet: w } } } = x;"],
  ["AudioContext.audioWorklet", "const [{ audioWorklet }] = xs;"],
  ["AudioContext.audioWorklet", "const { list: [{ audioWorklet }] } = x;"],
  ["AudioContext.audioWorklet", "let w; [{ audioWorklet: w }] = xs;"],
  ["AudioContext.audioWorklet", "let w; ({ list: [{ audioWorklet: w }] } = x);"],
  ["AudioContext.audioWorklet", "let w; [{ list: [{ audioWorklet: w }] }] = xs;"],
  ["AudioContext.audioWorklet", "let w; ({ a: { list: [[{ audioWorklet: w }]] } } = x);"],
  ["caches", "const c = await caches.open('k');"],
  ["caches", "await window.caches.keys();"],
  ["caches", "const { caches } = globalThis;"],
  ["randomUUID", "const { randomUUID: r } = crypto;"],
  ["randomUUID", "crypto?.['randomUUID']();"],
];

test.each(STRUCTURAL)("structural spelling of %s is a violation (positive control): %s", (id, code) => {
  const v = scanSecureContext("src/modules/x/src/a.ts", code + "\n");
  expect(v.map((x) => x.rule.id)).toContain(id);
});

test("a multi-line access reports the line of the member, not of the owner", () => {
  const v = scanSecureContext("src/a.ts", "const a = 1;\nnavigator\n  .mediaDevices;\n");
  expect(v.filter((x) => x.rule.id === "navigator.mediaDevices").map((x) => x.line)).toEqual([3]);
});

test("the seams may use their members in any spelling", () => {
  expect(scanSecureContext(MIC_SEAM_FILE, "const { mediaDevices } = navigator; ctx?.['audioWorklet'];")).toEqual([]);
  expect(scanSecureContext("src/client/shell/src/lib/deviceSignals.ts", "(navigator as unknown as Record<string, unknown>).deviceMemory;")).toEqual([]);
  expect(scanSecureContext(SEAM_FILE, "const { randomUUID } = crypto;")).toEqual([]);
});

test("look-alikes that are not the banned members do not match", () => {
  const ok = [
    "const caches = new Map(); caches.set('a', 1);",
    "class A { caches = 1; m() { return this.caches; } }",
    "const o = { caches: 1 }; o.caches;",
    "function f(caches: number) { return caches; }",
    "import { caches } from './x'; caches;",
    "interface I { audioWorklet?: unknown; mediaDevices?: unknown }",
    "const o = { audioWorklet: 1, mediaDevices: 2 };",
    "navigator.hardwareConcurrency; const { hardwareConcurrency } = navigator;",
    "const nav = { clipboard: 1 }; nav.clipboard;",
    "other.subtle; const { subtle } = other;",
    "const { locks } = registry;",
  ];
  for (const code of ok) {
    expect(scanSecureContext("src/a.ts", code + "\n"), code).toEqual([]);
  }
});

test("a null-owner member is banned in any destructuring and any access, on any object", () => {
  expect(scanSecureContext("src/a.ts", "const { audioWorklet } = anything;").map((v) => v.rule.id)).toEqual(["AudioContext.audioWorklet"]);
  expect(scanSecureContext("src/a.ts", "function f({ audioWorklet }) {}").map((v) => v.rule.id)).toEqual(["AudioContext.audioWorklet"]);
});

test("the documented BOUND: alias-shaped and non-syntactic reads are not caught", () => {
  const undecidable = [
    "const nav = navigator; nav.mediaDevices;",
    "function f(n: Navigator) { return n.mediaDevices; }",
    "Reflect.get(navigator, 'mediaDevices');",
    "'mediaDevices' in navigator;",
  ];
  for (const code of undecidable) expect(scanSecureContext("src/a.ts", code + "\n").filter((v) => v.rule.id === "navigator.mediaDevices"), code).toHaveLength(0);
});

test("non-literal keys on navigator/crypto are counted, and an alias is the documented bound", () => {
  const r = scanSecureContextDetailed("src/a.ts", "const k = 'mediaDevices';\nnavigator[k];\nglobalThis.crypto[k];\nconst nav = navigator;\nnav.mediaDevices;\n");
  expect(r.dynamic).toEqual([2, 3]);
  // The alias access is NOT caught: this pins the bound the gate header states.
  expect(r.violations).toEqual([]);
});

// ---- Svelte: script blocks and template expressions.
test("svelte: spellings in a script block are caught at the right file line", () => {
  const text = "<h1>x</h1>\n<script lang=\"ts\">\n  const a = 1;\n  const { clipboard } = navigator;\n</script>\n";
  expect(scanSecureContext("src/modules/x/src/A.svelte", text).map((v) => [v.line, v.rule.id])).toEqual([[4, "navigator.clipboard"]]);
});

test("svelte: template expressions, block openers and directives are parsed", () => {
  const cases = [
    ["{navigator['clipboard']}", "navigator.clipboard"],
    ["{@html navigator?.mediaDevices}", "navigator.mediaDevices"],
    ["{#if window.caches}x{/if}", "caches"],
    ["<button onclick={() => navigator['share']({})}>b</button>", "navigator.share"],
    ["{#each ctx.audioWorklet.items as item (item.id)}{item}{/each}", "AudioContext.audioWorklet"],
    ["<div class:on={navigator?.['locks']} />", "navigator.locks"],
    ["{#await navigator['gpu'] then a}{a}{:catch e}{e}{/await}", "navigator.gpu"],
  ];
  for (const [markup, id] of cases) {
    const v = scanSecureContext("src/modules/x/src/A.svelte", `<script>let a;</script>\n${markup}\n`);
    expect(v.map((x) => x.rule.id), markup).toContain(id);
    expect(v.find((x) => x.rule.id === id).line, markup).toBe(2);
  }
});

test("svelte: style blocks and closing tags contribute nothing", () => {
  const { regions, unparsedLines } = svelteRegions("<style>.a { color: red; }</style>\n<p>{/if}</p>{:else}\n");
  expect(regions).toEqual([]);
  expect(unparsedLines).toEqual([]);
});

// ---- The gate's entry point.
test("runGate: an unlistable tree fails loudly instead of scanning nothing", () => {
  const errors = [];
  const code = runGate({ listFiles: () => { throw new Error("could not determine the file list (`git ls-files` failed: boom)"); }, readFile: () => "", log: () => {}, error: (m) => errors.push(m) });
  expect(code).toBe(2);
  expect(errors).toEqual(["lint:secure-context: could not determine the file list (`git ls-files` failed: boom)"]);
});

test("runGate: a violation exits 1 with file:line and a clean tree exits 0, printing distinct allowed files", () => {
  const files = { "src/client/core/src/uuid.ts": "export const x = 1;", "src/modules/x/src/A.svelte": "<script>const { clipboard } = navigator;</script>" };
  const run = (contents) => {
    const out = { log: [], error: [] };
    const code = runGate({ listFiles: () => Object.keys(contents), readFile: (p) => contents[p], log: (m) => out.log.push(m), error: (m) => out.error.push(m) });
    return { code, ...out };
  };
  const bad = run(files);
  expect(bad.code).toBe(1);
  expect(bad.error[0]).toMatch(/src\/modules\/x\/src\/A\.svelte:1: `navigator\.clipboard`/);
  const clean = run({ ...files, "src/modules/x/src/A.svelte": "<script>const a = 1;</script>" });
  expect(clean.code).toBe(0);
  expect(clean.log.at(-1)).toContain(`${allowedFileSet().size} distinct allowed file(s)`);
  expect(allowedFileSet().size).toBe(new Set(RULES.flatMap((r) => r.allowedFiles)).size);
});

// ---- Svelte: a binding declared in ANY region of the file shadows a `globals` rule.
test("svelte: a script-declared `caches` is not the global in the template", () => {
  const text = "<script>let caches = $state([]);</script>\n{#each caches as c}{c}{/each}\n{caches.length}\n";
  expect(scanSecureContext("src/modules/x/src/A.svelte", text)).toEqual([]);
});

test("svelte: a module script's binding shadows the global in the instance script and the template", () => {
  const text = "<script module>export const caches = new Map();</script>\n<script>const n = caches.size;</script>\n{caches.size}\n";
  expect(scanSecureContext("src/modules/x/src/A.svelte", text)).toEqual([]);
});

test("svelte: template-declared names shadow the global; an undeclared use still reports it", () => {
  expect(scanSecureContext("src/modules/x/src/A.svelte", "<script>let items = [];</script>\n{#each items as caches}{caches}{/each}\n")).toEqual([]);
  expect(scanSecureContext("src/modules/x/src/A.svelte", "<script>let items = [];</script>\n{@const caches = items}{caches}\n")).toEqual([]);
  const v = scanSecureContext("src/modules/x/src/A.svelte", "<script>let items = [];</script>\n{#each items as i}{caches}{/each}\n");
  expect(v.map((x) => [x.line, x.rule.id])).toEqual([[2, "caches"]]);
});

// ---- Fail closed on every parse failure, not only template fragments.
test("a .ts file with a syntax error is an unread region, never a silent pass", () => {
  const r = scanSecureContextDetailed("src/a.ts", "const a = 1;\nconst b = ;\n");
  expect(r.unparsedLines).toEqual([2]);
});

test("a .svelte script block with a syntax error is reported at its file line", () => {
  const r = scanSecureContextDetailed("src/modules/x/src/A.svelte", "<h1>x</h1>\n<script>\n  const a = 1;\n  const b = ;\n</script>\n");
  expect(r.unparsedLines).toEqual([4]);
});

test("runGate: an unparsable file fails the gate", () => {
  const files = { "src/client/core/src/uuid.ts": "export const x = ;", "src/modules/x/src/A.svelte": "<script>const a = 1;</script>" };
  const errors = [];
  const code = runGate({ listFiles: () => Object.keys(files), readFile: (p) => files[p], log: () => {}, error: (m) => errors.push(m) });
  expect(code).toBe(1);
  expect(errors[0]).toMatch(/cannot parse src\/client\/core\/src\/uuid\.ts:1/);
});

// ---- `{@attach}` (Svelte 5) carries an expression like any other tag.
test("svelte: {@attach} expressions are parsed and scanned", () => {
  const ok = svelteRegions("<div {@attach (node) => { node.focus(); }}></div>\n");
  expect(ok.unparsedLines).toEqual([]);
  const v = scanSecureContext("src/modules/x/src/A.svelte", "<script>let a;</script>\n<div {@attach (n) => navigator['clipboard']}></div>\n");
  expect(v.map((x) => [x.line, x.rule.id])).toEqual([[2, "navigator.clipboard"]]);
});

// ---- The file-wide declared-name union counts VALUE bindings only.
test.each([
  ["a parameter of a function type", "type Probe = (caches: number) => void;\ncaches.open('x');\n"],
  ["a parameter of a method signature", "interface Api { open(caches: string): void }\ncaches.open('x');\n"],
  ["a type-only default import", "import type caches from './c';\ncaches.open('x');\n"],
  ["a type-only named import", "import { type caches } from './c';\ncaches.open('x');\n"],
  ["a type-only namespace import", "import type * as caches from './c';\ncaches.open('x');\n"],
])("%s does not shadow the global `caches`", (_name, code) => {
  const v = scanSecureContext("src/modules/x/src/a.ts", code);
  expect(v.map((x) => x.rule.id)).toContain("caches");
});

test.each([
  ["a value parameter", "function f(caches) { return caches.open('x'); }\n"],
  ["a value import", "import caches from './c';\ncaches.open('x');\n"],
  ["a named value import", "import { caches } from './c';\ncaches.open('x');\n"],
  ["a const", "const caches = new Map();\ncaches.open('x');\n"],
])("%s shadows the global `caches`", (_name, code) => {
  expect(scanSecureContext("src/modules/x/src/a.ts", code)).toEqual([]);
});

// ---- The corpus is the repository file list (tracked plus untracked, non-ignored).
const CORPUS_BASE = {
  "src/client/core/src/uuid.ts": "export const x = 1;\n",
  "src/modules/x/src/A.svelte": "<script>const a = 1;</script>\n",
};
const CORPUS_BANNED = "export const id = crypto.randomUUID();\n";
const SECURE_GATE = "scripts/check-secure-context-apis.mjs";

test("an untracked file naming a secure-context API fails the gate before git add", () => {
  const dir = makeRepo({ ...CORPUS_BASE, "src/modules/x/src/new.ts": CORPUS_BANNED }, Object.keys(CORPUS_BASE));
  const r = runScriptIn(dir, SECURE_GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("src/modules/x/src/new.ts");
});

test("the same file under an ignored path passes", () => {
  const dir = makeRepo(
    { ...CORPUS_BASE, ".gitignore": "ignored/\n", "src/modules/x/src/ignored/new.ts": CORPUS_BANNED },
    [...Object.keys(CORPUS_BASE), ".gitignore"],
  );
  expect(runScriptIn(dir, SECURE_GATE).status).toBe(0);
});

test("a tracked file deleted in the working tree does not throw ENOENT", () => {
  const dir = makeRepo({ ...CORPUS_BASE, "src/modules/x/src/gone.ts": "export const g = 1;\n" });
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "modules", "x", "src", "gone.ts"), join(dir, "gone-aside.ts"));
  const r = runScriptIn(dir, SECURE_GATE);
  expect(r.status).toBe(0);
  expect(r.stderr).not.toContain("ENOENT");
});

test("every allowed file exists, so an allow-list entry cannot outlive the file it names", () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  for (const file of [SEAM_FILE, HTTPS_SPEC_FILE, MIC_SEAM_FILE, ...allowedFileSet()]) {
    expect(existsSync(resolve(root, file)), file).toBe(true);
  }
});
