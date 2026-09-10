import { describe, it, expect, beforeEach } from "vitest";
import { basename, join } from "node:path";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import {
  findSkillFiles,
  extractApiRefs,
  apiRefResolves,
  checkSkillApiRefs,
} from "./check-skill-api-refs.mjs";

// One fixture root per describe block below, at a FIXED path rewritten in place rather than a
// fresh temp directory per run: this repo permits no permanent-deletion call, so a per-run
// directory would accumulate forever. Every test writes deterministic content into a fixed
// subdirectory, so re-running only ever overwrites what an earlier run left there.
const FIXTURE_ROOT = join(
  tmpdir(),
  `shadowcat-${basename(fileURLToPath(import.meta.url), ".test.mjs")}-fixture`,
);

describe("findSkillFiles", () => {
  const skills = (r) => join(r, ".claude", "skills");

  it("finds every SKILL.md recursively, ignoring same-named-differently files", () => {
    const root = join(FIXTURE_ROOT, "find-basic");
    mkdirSync(join(skills(root), "a"), { recursive: true });
    mkdirSync(join(skills(root), "b"), { recursive: true });
    writeFileSync(join(skills(root), "a", "SKILL.md"), "");
    writeFileSync(join(skills(root), "b", "SKILL.md"), "");
    writeFileSync(join(skills(root), "b", "skill.md"), ""); // wrong case — must not match
    writeFileSync(join(skills(root), "b", "SKILL.mdx"), ""); // wrong extension — must not match
    const { files } = findSkillFiles(skills(root), { trackedDirs: new Set(["a", "b"]) });
    expect(files).toEqual([
      join(skills(root), "a", "SKILL.md"),
      join(skills(root), "b", "SKILL.md"),
    ]);
  });

  // The corpus rule both gates now share: an untracked directory is vendored prose, and a
  // `/api/...` pointer inside one is not this repo's to fail CI on.
  it("skips an untracked skill directory and reports it as excluded", () => {
    const root = join(FIXTURE_ROOT, "find-untracked");
    mkdirSync(join(skills(root), "tracked"), { recursive: true });
    mkdirSync(join(skills(root), "vendored"), { recursive: true });
    writeFileSync(join(skills(root), "tracked", "SKILL.md"), "");
    writeFileSync(join(skills(root), "vendored", "SKILL.md"), "See `/api/ts/nope.html`.");
    const found = findSkillFiles(skills(root), {
      trackedDirs: new Set(["tracked"]),
      untrackedDirs: ["vendored"],
    });
    expect(found.files).toEqual([join(skills(root), "tracked", "SKILL.md")]);
    expect(found.untrackedDirs).toEqual(["vendored"]);
  });

  it("returns an empty array for a scope with no SKILL.md files", () => {
    const root = join(FIXTURE_ROOT, "find-empty");
    mkdirSync(join(skills(root), "empty"), { recursive: true });
    expect(findSkillFiles(skills(root), { trackedDirs: new Set(["empty"]) }).files).toEqual([]);
  });
});

describe("extractApiRefs", () => {
  it("extracts /api/rust/... and /api/ts/... citations, deep paths only", () => {
    const text = [
      "See `/api/rust/shadowcat/data/engine/token/` and `/api/ts/modules/_shadowcat_core.html`.",
      "The generated API root is `/api/ts/` (TypeDoc), rustdoc under `/api/rust/shadowcat/`.",
      "The public root `/api/rust/` itself has no index page.",
    ].join("\n");
    expect(extractApiRefs(text)).toEqual([
      "/api/rust/shadowcat/data/engine/token/",
      "/api/ts/modules/_shadowcat_core.html",
      "/api/rust/shadowcat/",
    ]);
  });

  it("does not match server HTTP routes that merely share the /api/ prefix", () => {
    const text = "`GET /api/login`, `/api/me`, `/api/worlds/{id}/schemas`, `/api/admin/backup`.";
    expect(extractApiRefs(text)).toEqual([]);
  });

  it("returns an empty array when there are no /api/... citations at all", () => {
    expect(extractApiRefs("No doc pointers here.")).toEqual([]);
  });
});

describe("apiRefResolves", () => {
  const root = join(FIXTURE_ROOT, "api-ref-resolves");
  beforeEach(() => {
    mkdirSync(join(root, "api", "ts", "modules"), { recursive: true });
    writeFileSync(join(root, "api", "ts", "modules", "_shadowcat_core.html"), "<html></html>");
    mkdirSync(join(root, "api", "rust", "shadowcat", "data"), { recursive: true });
    writeFileSync(join(root, "api", "rust", "shadowcat", "data", "index.html"), "<html></html>");
  });

  it("resolves a TypeDoc .html page path", () => {
    expect(apiRefResolves(root, "/api/ts/modules/_shadowcat_core.html")).toBe(true);
  });
  it("resolves a trailing-slash rustdoc module directory via its index.html", () => {
    expect(apiRefResolves(root, "/api/rust/shadowcat/data/")).toBe(true);
  });
  it("does not resolve a page that was never generated", () => {
    expect(apiRefResolves(root, "/api/ts/modules/_shadowcat_nonexistent.html")).toBe(false);
  });
  it("does not resolve a rustdoc directory with no index.html", () => {
    expect(apiRefResolves(root, "/api/rust/shadowcat/nonexistent/")).toBe(false);
  });
});

describe("checkSkillApiRefs", () => {
  const repoRoot = join(FIXTURE_ROOT, "check-api-refs-repo");
  const skillsRoot = join(repoRoot, ".claude", "skills");
  const distDocsRoot = join(FIXTURE_ROOT, "check-api-refs-dist-docs");
  const scoped = { trackedDirs: new Set(["shadowcat-codebase-example"]) };
  beforeEach(() => {
    mkdirSync(skillsRoot, { recursive: true });
    mkdirSync(join(distDocsRoot, "api", "ts", "modules"), { recursive: true });
    writeFileSync(join(distDocsRoot, "api", "ts", "modules", "_shadowcat_core.html"), "<html></html>");
  });

  it("stamps the result with what was scanned and reports zero broken pointers when every citation resolves", () => {
    mkdirSync(join(skillsRoot, "shadowcat-codebase-example"), { recursive: true });
    writeFileSync(
      join(skillsRoot, "shadowcat-codebase-example", "SKILL.md"),
      "See `/api/ts/modules/_shadowcat_core.html`.",
    );
    const result = checkSkillApiRefs(skillsRoot, distDocsRoot, scoped);
    expect(result.filesScanned).toBe(1);
    expect(result.refsChecked).toBe(1);
    expect(result.broken).toEqual([]);
  });

  // Non-vacuity proof: pointing a skill at a path that does not exist on the assembled site
  // must be reported broken, not silently pass.
  it("reports a broken pointer when a skill cites a path that does not exist on the site", () => {
    mkdirSync(join(skillsRoot, "shadowcat-codebase-example"), { recursive: true });
    const skillFile = join(skillsRoot, "shadowcat-codebase-example", "SKILL.md");
    writeFileSync(skillFile, "See `/api/ts/modules/_shadowcat_totally_made_up.html`.");
    const result = checkSkillApiRefs(skillsRoot, distDocsRoot, scoped);
    expect(result.broken).toEqual([
      { file: skillFile, ref: "/api/ts/modules/_shadowcat_totally_made_up.html" },
    ]);
  });

  // Non-vacuity proof: a scan whose extraction pattern matches nothing must be distinguishable
  // from a scan that verified real citations and found them all clean — `refsChecked` is the
  // signal a caller checks for that, and it stays 0 here on purpose.
  it("reports zero refsChecked (not a clean pass) when no skill cites an /api/... path", () => {
    mkdirSync(join(skillsRoot, "shadowcat-codebase-example"), { recursive: true });
    writeFileSync(
      join(skillsRoot, "shadowcat-codebase-example", "SKILL.md"),
      "No generated-doc pointers in this skill at all.",
    );
    const result = checkSkillApiRefs(skillsRoot, distDocsRoot, scoped);
    expect(result.filesScanned).toBe(1);
    expect(result.refsChecked).toBe(0);
    expect(result.broken).toEqual([]);
  });
});
