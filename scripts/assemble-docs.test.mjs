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
    writeFileSync(join(root, "guides", "index.html"), "<html></html>");
    writeFileSync(join(root, "index.html"), `<a href="guides/">dir</a>`);
    expect(checkLinks(root, [join(root, "index.html")])).toEqual([]);
  });
  it("reports a directory link whose index.html is missing", () => {
    writeFileSync(join(root, "index.html"), `<a href="guides/">dir</a>`);
    const broken = checkLinks(root, [join(root, "index.html")]);
    expect(broken).toHaveLength(1);
    expect(broken[0].target).toContain("index.html");
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
