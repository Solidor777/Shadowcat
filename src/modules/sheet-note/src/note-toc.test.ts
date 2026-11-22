import { describe, it, expect } from "vitest";
import { extractHeadings, assignHeadingIds, findAnchor } from "./note-toc";

describe("extractHeadings", () => {
  it("reads nested ATX headings with their levels and text", () => {
    const entries = extractHeadings("# One\ntext\n## Two ##\n### Three\n###### Six");
    expect(entries).toEqual([
      { slug: "one", text: "One", level: 1 },
      { slug: "two", text: "Two", level: 2 },
      { slug: "three", text: "Three", level: 3 },
      { slug: "six", text: "Six", level: 6 },
    ]);
  });

  it("skips a # line inside a fenced code block, backtick or tilde", () => {
    const source = "```\n# not a heading\n```\n# Real\n~~~\n## also not\n~~~\n";
    expect(extractHeadings(source).map((e) => e.text)).toEqual(["Real"]);
  });

  it("closes a fence only on the same character with at least the opener's length", () => {
    const source = "````\n```\n# inside\n~~~~\n````\n# After";
    expect(extractHeadings(source).map((e) => e.text)).toEqual(["After"]);
  });

  it("de-duplicates colliding slugs with a numeric suffix that never re-collides", () => {
    const entries = extractHeadings("# Intro\n# Intro\n# Intro 1\n# Intro");
    expect(entries.map((e) => e.slug)).toEqual(["intro", "intro-1", "intro-1-1", "intro-2"]);
  });

  it("normalises punctuation and falls back for a heading with no alphanumerics", () => {
    const entries = extractHeadings("# Hello, World!\n# ???\n#");
    expect(entries.map((e) => e.slug)).toEqual(["hello-world", "section", "section-1"]);
  });

  it("ignores a # with no following space and a line indented four spaces", () => {
    expect(extractHeadings("#hashtag\n    # code")).toEqual([]);
  });

  it("only ever yields slugs inside the server's fragment alphabet", () => {
    for (const e of extractHeadings("# Ünï côdé ☃\n# a_b c")) {
      expect(e.slug).toMatch(/^[A-Za-z0-9_-]+$/);
    }
  });
});

describe("assignHeadingIds", () => {
  it("assigns ids positionally to the rendered headings", () => {
    const root = document.createElement("div");
    root.innerHTML = "<h1>One</h1><p>x</p><h2>Two</h2><h2>Three</h2>";
    assignHeadingIds(root, extractHeadings("# One\n## Two\n## Three"));
    expect([...root.querySelectorAll("h1,h2")].map((h) => h.id)).toEqual(["one", "two", "three"]);
  });

  it("skips a rendered heading whose level the source scan did not see", () => {
    const root = document.createElement("div");
    root.innerHTML = "<h3>Quoted</h3><h1>One</h1><h2>Two</h2>";
    assignHeadingIds(root, extractHeadings("> ### Quoted\n# One\n## Two"));
    expect([...root.querySelectorAll("h1,h2,h3")].map((h) => h.id)).toEqual(["", "one", "two"]);
  });

  it("leaves surplus rendered headings without an id", () => {
    const root = document.createElement("div");
    root.innerHTML = "<h1>One</h1><h1>Two</h1>";
    assignHeadingIds(root, extractHeadings("# One"));
    expect([...root.querySelectorAll("h1")].map((h) => h.id)).toEqual(["one", ""]);
  });
});

describe("findAnchor", () => {
  it("finds an id only inside the container", () => {
    const outside = document.createElement("h1");
    outside.id = "intro";
    document.body.append(outside);
    const root = document.createElement("div");
    root.innerHTML = '<h1 id="intro">In</h1>';
    expect(findAnchor(root, "intro")).toBe(root.firstElementChild);
    expect(findAnchor(root, "missing")).toBeNull();
    outside.remove();
  });
});
