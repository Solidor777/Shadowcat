import { describe, it, expect, beforeEach } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  extractLocalLinks,
  checkLinks,
  assemble,
  toRelativeHref,
  htmlFilesUnder,
  cssFilesUnder,
} from "./assemble-docs.mjs";

describe("extractLocalLinks", () => {
  it("returns local href/src targets, skipping schemes and fragments", () => {
    const html = `<a href="guides/hosting.html">g</a>
      <a href="https://example.com/x">ext</a>
      <a href="#anchor">frag</a>
      <img src="../logo.png">
      <a href="mailto:testuser-01@example.com">m</a>
      <a href="//cdn.example/lib.js">protorel</a>
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

describe("toRelativeHref", () => {
  it("rewrites a root-absolute asset for a depth-0 page", () => {
    expect(toRelativeHref("/assets/style.css", 0)).toBe("./assets/style.css");
  });

  it("walks up one level for a depth-1 page", () => {
    expect(toRelativeHref("/assets/style.css", 1)).toBe("../assets/style.css");
  });

  it("walks up two levels for a depth-2 page", () => {
    expect(toRelativeHref("/assets/style.css", 2)).toBe("../../assets/style.css");
  });

  it("expands a directory target to its index.html", () => {
    expect(toRelativeHref("/modules/", 1)).toBe("../modules/index.html");
  });

  it("expands the site root to index.html", () => {
    expect(toRelativeHref("/", 0)).toBe("./index.html");
  });

  it("leaves scheme-prefixed URLs alone", () => {
    expect(toRelativeHref("https://example.com/x", 2)).toBe("https://example.com/x");
  });

  it("leaves protocol-relative URLs alone", () => {
    expect(toRelativeHref("//example.com/x", 1)).toBe("//example.com/x");
  });

  it("leaves fragment-only links alone", () => {
    expect(toRelativeHref("#VPContent", 1)).toBe("#VPContent");
  });

  it("leaves an already-relative link alone", () => {
    expect(toRelativeHref("./nested/page.html", 1)).toBe("./nested/page.html");
  });

  it("preserves a fragment on a rewritten link", () => {
    expect(toRelativeHref("/protocol.html#frames", 1)).toBe("../protocol.html#frames");
  });
});

describe("htmlFilesUnder", () => {
  let root;
  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "docs-walk-"));
    mkdirSync(join(root, "guides"), { recursive: true });
    mkdirSync(join(root, "api", "ts"), { recursive: true });
    writeFileSync(join(root, "index.html"), "<html></html>");
    writeFileSync(join(root, "guides", "a.html"), "<html></html>");
    writeFileSync(join(root, "guides", "a.css"), "body{}");
    writeFileSync(join(root, "api", "ts", "index.html"), "<html></html>");
  });

  it("recursively lists .html files under dir", () => {
    const found = htmlFilesUnder(root).sort();
    expect(found).toEqual(
      [join(root, "api", "ts", "index.html"), join(root, "guides", "a.html"), join(root, "index.html")].sort(),
    );
  });

  it("skips the given top-level subtrees", () => {
    const found = htmlFilesUnder(root, [join("api", "ts")]).sort();
    expect(found).toEqual([join(root, "guides", "a.html"), join(root, "index.html")].sort());
  });
});

describe("cssFilesUnder", () => {
  let root;
  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "docs-walk-css-"));
    mkdirSync(join(root, "assets"), { recursive: true });
    mkdirSync(join(root, "api", "ts"), { recursive: true });
    writeFileSync(join(root, "assets", "style.css"), "body{}");
    writeFileSync(join(root, "assets", "style.css.map"), "{}");
    writeFileSync(join(root, "api", "ts", "style.css"), "body{}");
  });

  it("recursively lists .css files under dir", () => {
    const found = cssFilesUnder(root).sort();
    expect(found).toEqual([join(root, "api", "ts", "style.css"), join(root, "assets", "style.css")].sort());
  });

  it("skips the given top-level subtrees", () => {
    const found = cssFilesUnder(root, [join("api", "ts")]);
    expect(found).toEqual([join(root, "assets", "style.css")]);
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
