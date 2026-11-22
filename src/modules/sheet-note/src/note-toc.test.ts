import { describe, it, expect } from "vitest";
import { collectHeadings, findAnchor } from "./note-toc";

/** Builds a container holding `html`. */
function rendered(html: string): HTMLElement {
  const root = document.createElement("div");
  root.innerHTML = html;
  return root;
}

describe("collectHeadings", () => {
  it("reads every heading with its level and text, and assigns the ids", () => {
    const root = rendered("<h1>One</h1><p>x</p><h2>Two</h2><h3>Three</h3><h6>Six</h6>");
    expect(collectHeadings(root)).toEqual([
      { slug: "one", text: "One", level: 1 },
      { slug: "two", text: "Two", level: 2 },
      { slug: "three", text: "Three", level: 3 },
      { slug: "six", text: "Six", level: 6 },
    ]);
    expect([...root.querySelectorAll("h1,h2,h3,h6")].map((h) => h.id)).toEqual(["one", "two", "three", "six"]);
  });

  it("includes headings in any position, such as inside a blockquote or list", () => {
    const root = rendered("<blockquote><h3>Quoted</h3></blockquote><ul><li><h2>Listed</h2></li></ul><h1>Top</h1>");
    expect(collectHeadings(root).map((e) => e.text)).toEqual(["Quoted", "Listed", "Top"]);
  });

  it("uses the text content of nested inline markup", () => {
    const root = rendered("<h2>Raw <em>part</em> <code>2</code></h2>");
    expect(collectHeadings(root)[0]).toEqual({ slug: "raw-part-2", text: "Raw part 2", level: 2 });
  });

  it("de-duplicates colliding slugs with a numeric suffix that never re-collides", () => {
    const root = rendered("<h1>Intro</h1><h1>Intro</h1><h1>Intro 1</h1><h1>Intro</h1>");
    expect(collectHeadings(root).map((e) => e.slug)).toEqual(["intro", "intro-1", "intro-1-1", "intro-2"]);
  });

  it("normalises punctuation and falls back for a heading with no alphanumerics", () => {
    const root = rendered("<h1>Hello, World!</h1><h1>???</h1><h1></h1>");
    expect(collectHeadings(root).map((e) => e.slug)).toEqual(["hello-world", "section", "section-1"]);
  });

  it("only ever yields slugs inside the server's fragment alphabet", () => {
    for (const e of collectHeadings(rendered("<h1>Ünï côdé ☃</h1><h1>a_b c</h1>"))) {
      expect(e.slug).toMatch(/^[A-Za-z0-9_-]+$/);
    }
  });

  it("returns nothing for a body without headings", () => {
    expect(collectHeadings(rendered("<p>plain</p>"))).toEqual([]);
  });
});

describe("findAnchor", () => {
  it("finds an id only inside the container", () => {
    const outside = document.createElement("h1");
    outside.id = "intro";
    document.body.append(outside);
    const root = rendered('<h1 id="intro">In</h1>');
    expect(findAnchor(root, "intro")).toBe(root.firstElementChild);
    expect(findAnchor(root, "missing")).toBeNull();
    outside.remove();
  });
});
