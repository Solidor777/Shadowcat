import { test, expect } from "vitest";
import { scanRoutePrefix, isCovered, assertGateLive, runGate, literalSpans, SEAM_FILE } from "./check-route-prefix-literals.mjs";

const lines = (path, text) => scanRoutePrefix(path, text).map((v) => v.line);

test("a bare /api literal in each string form is a violation", () => {
  expect(lines("src/a.ts", 'fetch("/api/me");\n')).toEqual([1]);
  expect(lines("src/a.ts", "fetch('/api/me');\n")).toEqual([1]);
  expect(lines("src/a.ts", "fetch(`/api/worlds/${id}`);\n")).toEqual([1]);
  expect(lines("src/a.ts", "const u = `/api/me`;\n")).toEqual([1]);
  expect(lines("src/a.ts", 'const x = 1;\nfetch("/api");\n')).toEqual([2]);
});

test("a bare /modules and /ws literal is a violation", () => {
  expect(lines("src/a.ts", 'import(entry ?? "/modules/m/index.js");\n')).toEqual([1]);
  expect(lines("src/a.ts", 'new WebSocket("/ws?world=w");\n')).toEqual([1]);
  expect(lines("src/a.ts", 'const s = "/ws";\n')).toEqual([1]);
});

test("a literal inside apiPath(...) passes, including a nested template", () => {
  expect(lines("src/a.ts", 'fetch(apiPath("/api/me"));\n')).toEqual([]);
  expect(lines("src/a.ts", "fetch(apiPath(`/api/trash${q ? `?${q}` : ''}`));\n")).toEqual([]);
  expect(lines("src/a.ts", 'fetch(apiPath(a ? "/api/x" : "/modules/y"));\n')).toEqual([]);
});

test("a literal that only mentions a path elsewhere does not match", () => {
  expect(lines("src/a.ts", 'const a = "GET /api/me";\n')).toEqual([]);
  expect(lines("src/a.ts", 'const a = "/apis/me";\n')).toEqual([]);
  expect(lines("src/a.ts", 'const a = "/wsx";\n')).toEqual([]);
  expect(lines("src/a.ts", 'const a = "https://x.example/api/me";\n')).toEqual([]);
});

test("comments and doc examples are never read", () => {
  expect(lines("src/a.ts", '// fetch("/api/me")\n/** @example fetch("/api/me") */\n')).toEqual([]);
});

test("an Error message that names a request is not a request", () => {
  expect(lines("src/a.ts", "throw new Error(`/api/me → ${s}`);\n")).toEqual([]);
  expect(lines("src/a.ts", 'const e = new Error("/api/config: bad");\n')).toEqual([]);
  expect(lines("src/a.ts", 'log("/api/me");\n')).toEqual([1]);
});

test("import specifiers and literal types are not requests", () => {
  expect(lines("src/a.ts", 'import x from "/modules/a.js";\ntype P = "/api/me";\n')).toEqual([]);
});

test("a Svelte script literal and a template attribute are violations; apiPath passes", () => {
  const bad = '<script lang="ts">\n  const r = fetch("/api/me");\n</script>\n<img src="/api/assets/a" alt="" />\n';
  expect(lines("src/a.svelte", bad)).toEqual([2, 4]);
  const ok = '<script lang="ts">\n  const r = fetch(apiPath("/api/me"));\n</script>\n<img src={url} alt="" />\n';
  expect(lines("src/a.svelte", ok)).toEqual([]);
  expect(lines("src/a.svelte", "<script>\n</script>\n<!-- <img src=\"/api/x\"> -->\n")).toEqual([]);
});

test("literalSpans bounds a template expression whole so a fixer can wrap it", () => {
  const code = "f(`/api/a/${x}/b`);";
  const [s] = literalSpans(code);
  expect(code.slice(s.start, s.end)).toBe("`/api/a/${x}/b`");
});

test("test, e2e, config and generated files are outside the gate", () => {
  expect(isCovered("src/client/core/src/user-rest.ts")).toBe(true);
  expect(isCovered("src/client/shell/src/App.svelte")).toBe(true);
  expect(isCovered("src/client/core/src/user-rest.test.ts")).toBe(false);
  expect(isCovered("src/client/shell/e2e/foo.spec.ts")).toBe(false);
  expect(isCovered("src/client/shell/e2e-support/lan-address.ts")).toBe(false);
  expect(isCovered("src/client/shell/vite.config.ts")).toBe(false);
  expect(isCovered("src/types/generated/Foo.ts")).toBe(false);
  expect(isCovered("src/server/src/x.js")).toBe(false);
  expect(isCovered("scripts/x.mjs")).toBe(false);
});

test("a scan that misses the seam file or every Svelte file is inert, not clean", () => {
  expect(() => assertGateLive([])).toThrow(/inert/);
  expect(() => assertGateLive(["src/a.ts", "src/b.svelte"])).toThrow(/base-path/);
  expect(() => assertGateLive([SEAM_FILE])).toThrow(/svelte/);
  expect(() => assertGateLive([SEAM_FILE, "src/b.svelte"])).not.toThrow();
});

test("runGate reports each offender and exits 1; a clean tree exits 0; a listing failure exits 2", () => {
  const files = { [SEAM_FILE]: "export const a = 1;\n", "src/x.svelte": "<script></script>\n", "src/bad.ts": 'fetch("/api/me");\n' };
  const out = [];
  const err = [];
  const io = (listFiles) => ({ listFiles, readFile: (p) => files[p], log: (m) => out.push(m), error: (m) => err.push(m) });
  expect(runGate(io(() => Object.keys(files)))).toBe(1);
  expect(err.join("\n")).toMatch(/src\/bad\.ts:1/);
  delete files["src/bad.ts"];
  err.length = 0;
  expect(runGate(io(() => Object.keys(files)))).toBe(0);
  expect(runGate(io(() => { throw new Error("no git"); }))).toBe(2);
});
