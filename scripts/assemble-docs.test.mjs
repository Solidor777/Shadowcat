import { describe, it, expect, beforeEach } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  extractLocalLinks,
  checkLinks,
  assemble,
  toRelativeHref,
  htmlFilesUnder,
  cssFilesUnder,
  rewriteAbsolutePaths,
  hasSurvivingAbsoluteRef,
  survivingAbsoluteRefs,
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

  it("does not expand a directory-shaped query value on a file target", () => {
    expect(toRelativeHref("/search?redirect=/home/", 1)).toBe("../search?redirect=/home/");
  });

  it("expands a directory target while preserving its query", () => {
    expect(toRelativeHref("/modules/?foo=1", 1)).toBe("../modules/index.html?foo=1");
  });

  it("preserves a query value that itself contains a slash", () => {
    expect(toRelativeHref("/download?path=/a/b", 1)).toBe("../download?path=/a/b");
  });

  it("preserves both a query and a fragment, in order", () => {
    expect(toRelativeHref("/page?tab=info#section", 1)).toBe("../page?tab=info#section");
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

describe("rewriteAbsolutePaths", () => {
  let root;
  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "docs-rewrite-"));
  });

  it("rewrites root-absolute href/src at depth 0, leaving an already-relative ref alone", () => {
    const file = join(root, "index.html");
    writeFileSync(
      file,
      `<link href="/assets/style.css"><a href="./guides/a.html">g</a><img src="/logo.png">`,
    );
    const changed = rewriteAbsolutePaths(root, [file]);
    expect(changed).toBe(1);
    expect(readFileSync(file, "utf8")).toBe(
      `<link href="./assets/style.css"><a href="./guides/a.html">g</a><img src="./logo.png">`,
    );
  });

  it("rewrites at depth 2, walking up two directory levels", () => {
    mkdirSync(join(root, "guides", "sub"), { recursive: true });
    const file = join(root, "guides", "sub", "b.html");
    writeFileSync(file, `<a href="/protocol.html">p</a>`);
    rewriteAbsolutePaths(root, [file]);
    expect(readFileSync(file, "utf8")).toBe(`<a href="../../protocol.html">p</a>`);
  });

  it("rewrites CSS url() references for each quote style at depth 1", () => {
    mkdirSync(join(root, "assets"), { recursive: true });
    const file = join(root, "assets", "style.css");
    writeFileSync(
      file,
      `.a{background:url(/assets/a.png)}.b{background:url('/assets/b.png')}.c{background:url("/assets/c.png")}`,
    );
    const changed = rewriteAbsolutePaths(root, [file]);
    expect(changed).toBe(1);
    expect(readFileSync(file, "utf8")).toBe(
      `.a{background:url(../assets/a.png)}.b{background:url('../assets/b.png')}.c{background:url("../assets/c.png")}`,
    );
  });

  it("returns 0 and leaves the file untouched when nothing needs rewriting", () => {
    const file = join(root, "index.html");
    const before = `<a href="./local.html">l</a>`;
    writeFileSync(file, before);
    const changed = rewriteAbsolutePaths(root, [file]);
    expect(changed).toBe(0);
    expect(readFileSync(file, "utf8")).toBe(before);
  });

  it("counts only the files it actually changes", () => {
    const changedFile = join(root, "a.html");
    const untouchedFile = join(root, "b.html");
    writeFileSync(changedFile, `<a href="/protocol.html">p</a>`);
    writeFileSync(untouchedFile, `<a href="./local.html">l</a>`);
    expect(rewriteAbsolutePaths(root, [changedFile, untouchedFile])).toBe(1);
  });
});

// The docs build fails on a non-empty `survivingAbsoluteRefs`, so both halves of that gate are
// observed directly rather than only through the entry block that consumes them: the
// `hasSurvivingAbsoluteRef` predicate against adversarial HTML and CSS below, and the
// `survivingAbsoluteRefs` scan that turns it into the build's exit status.
describe("hasSurvivingAbsoluteRef", () => {
  it("HTML: reports a surviving root-absolute href/src", () => {
    expect(hasSurvivingAbsoluteRef("page.html", `<a href="/protocol.html">p</a>`)).toBe(true);
    expect(hasSurvivingAbsoluteRef("page.html", `<img src='/logo.png'>`)).toBe(true);
  });
  it("HTML: clean once every ref is relative or scheme-prefixed", () => {
    expect(
      hasSurvivingAbsoluteRef(
        "page.html",
        `<a href="./guides/a.html">g</a><a href="https://example.com/x">ext</a><a href="//cdn.example/lib.js">p</a>`,
      ),
    ).toBe(false);
  });
  it("CSS: reports a surviving root-absolute url()", () => {
    expect(hasSurvivingAbsoluteRef("style.css", `.a{background:url(/assets/a.png)}`)).toBe(true);
    expect(hasSurvivingAbsoluteRef("style.css", `.b{background:url( '/assets/b.png' )}`)).toBe(true);
  });
  it("CSS: clean once every url() is relative or scheme-prefixed", () => {
    expect(
      hasSurvivingAbsoluteRef(
        "style.css",
        `.a{background:url(../assets/a.png)}.b{background:url("https://cdn.example/x.png")}`,
      ),
    ).toBe(false);
  });
  it("dispatches on the CSS extension rather than content, so a .html file's url() text is ignored", () => {
    expect(hasSurvivingAbsoluteRef("page.html", `<style>.a{background:url(/x.png)}</style>`)).toBe(
      false,
    );
  });
});

describe("survivingAbsoluteRefs", () => {
  it("reports the files whose content still carries a root-absolute ref, and only those", () => {
    const root = mkdtempSync(join(tmpdir(), "docs-survivors-"));
    const dirty = join(root, "dirty.html");
    const clean = join(root, "clean.html");
    const dirtyCss = join(root, "dirty.css");
    writeFileSync(dirty, `<a href="/protocol.html">p</a>`);
    writeFileSync(clean, `<a href="./protocol.html">p</a>`);
    writeFileSync(dirtyCss, `.a{background:url(/assets/a.png)}`);
    expect(survivingAbsoluteRefs([dirty, clean, dirtyCss])).toEqual([dirty, dirtyCss]);
  });

  it("returns an empty list when every file is clean", () => {
    const root = mkdtempSync(join(tmpdir(), "docs-survivors-clean-"));
    const page = join(root, "index.html");
    writeFileSync(page, `<a href="./local.html">l</a>`);
    expect(survivingAbsoluteRefs([page])).toEqual([]);
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
