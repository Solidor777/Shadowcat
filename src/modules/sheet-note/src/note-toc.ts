/** One heading found in a note's rendered body. */
export interface TocEntry {
  /** The anchor id: lowercase `a-z0-9-`, unique within the note. Always matches the server's
   * fragment-only link alphabet `[A-Za-z0-9_-]+`, so `[text](#slug)` survives sanitization. */
  slug: string;
  /** The heading's text content, trimmed. */
  text: string;
  /** The heading level, 1 (`h1`) to 6 (`h6`). */
  level: number;
}

/** The slug used when a heading's text has no alphanumeric character. */
const EMPTY_SLUG = "section";

/**
 * Builds a slug from heading text: lowercased, every run of non-alphanumeric characters becomes
 * one `-`, leading and trailing `-` trimmed.
 * @param text The heading text.
 * @returns The slug, or `"section"` when the text has no ASCII letter or digit.
 * @example
 * ```ts
 * slugify("Hello, World!"); // "hello-world"
 * ```
 */
function slugify(text: string): string {
  const slug = text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug === "" ? EMPTY_SLUG : slug;
}

/**
 * Reads every `h1` to `h6` element in the rendered note body, in document order, gives each its
 * anchor id and returns the matching entries. The rendered elements are the only source, so the
 * list and the ids cannot disagree for any heading syntax (ATX, setext, raw HTML, nested in a
 * quote or list). Colliding slugs get a numeric suffix (`intro`, `intro-1`, `intro-2`), and a
 * suffixed slug never collides with a heading whose own text already slugs to it.
 * @param container The element holding the rendered note body.
 * @returns The headings in document order.
 * @example
 * ```ts
 * declare const bodyElement: Element;
 * const entries = collectHeadings(bodyElement);
 * ```
 */
export function collectHeadings(container: Element): TocEntry[] {
  const entries: TocEntry[] = [];
  const used = new Set<string>();
  for (const el of container.querySelectorAll("h1, h2, h3, h4, h5, h6")) {
    const text = (el.textContent ?? "").trim();
    const base = slugify(text);
    let slug = base;
    for (let n = 1; used.has(slug); n += 1) slug = `${base}-${n}`;
    used.add(slug);
    el.id = slug;
    entries.push({ slug, text, level: Number(el.tagName.slice(1)) });
  }
  return entries;
}

/**
 * Finds the element in `container` whose id is `slug`. The lookup compares ids directly, so it
 * is scoped to the container and never reaches a same-named id in another open sheet.
 * @param container The element holding the rendered note body.
 * @param slug The anchor id.
 * @returns The element, or `null` when none carries the id.
 * @example
 * ```ts
 * declare const bodyElement: Element;
 * findAnchor(bodyElement, "intro")?.scrollIntoView();
 * ```
 */
export function findAnchor(container: Element, slug: string): Element | null {
  for (const el of container.querySelectorAll("[id]")) {
    if (el.id === slug) return el;
  }
  return null;
}
