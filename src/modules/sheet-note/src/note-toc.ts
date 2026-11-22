/** One heading found in a note's raw markdown source. */
export interface TocEntry {
  /** The anchor id: lowercase `a-z0-9-`, unique within the note. Always matches the server's
   * fragment-only link alphabet `[A-Za-z0-9_-]+`, so `[text](#slug)` survives sanitization. */
  slug: string;
  /** The heading text with its ATX markers and closing hashes removed. */
  text: string;
  /** The heading level, 1 (`#`) to 6 (`######`). */
  level: number;
}

/** An ATX heading line: up to three spaces of indent, one to six `#`, then a space/tab or end of line. */
const ATX_HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*$/;
/** A fenced-code-block delimiter line: up to three spaces of indent, then three or more backticks or tildes. */
const FENCE = /^ {0,3}(`{3,}|~{3,})/;
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
 * Scans a note's markdown source line by line for ATX headings, skipping lines inside fenced
 * code blocks. A fence closes only on a line of the same character with at least as many
 * characters as the opener. Colliding slugs get a numeric suffix (`intro`, `intro-1`, `intro-2`),
 * and a suffixed slug never collides with a heading whose own text already slugs to it.
 * @param source The note's raw markdown source.
 * @returns The headings in document order.
 * @example
 * ```ts
 * extractHeadings("# Intro\n## Intro");
 * // [{ slug: "intro", text: "Intro", level: 1 }, { slug: "intro-1", text: "Intro", level: 2 }]
 * ```
 */
export function extractHeadings(source: string): TocEntry[] {
  const entries: TocEntry[] = [];
  const used = new Set<string>();
  let fence: string | null = null;
  for (const line of source.split(/\r?\n/)) {
    const fenceMatch = FENCE.exec(line);
    if (fence !== null) {
      if (fenceMatch && fenceMatch[1].startsWith(fence[0]) && fenceMatch[1].length >= fence.length && line.trim() === fenceMatch[1]) {
        fence = null;
      }
      continue;
    }
    if (fenceMatch) {
      fence = fenceMatch[1];
      continue;
    }
    const heading = ATX_HEADING.exec(line);
    if (!heading) continue;
    const text = (heading[2] ?? "").replace(/[ \t]+#+$/, "").replace(/^#+$/, "").trim();
    const base = slugify(text);
    let slug = base;
    for (let n = 1; used.has(slug); n += 1) slug = `${base}-${n}`;
    used.add(slug);
    entries.push({ slug, text, level: heading[1].length });
  }
  return entries;
}

/**
 * Assigns each rendered heading element its anchor id, in document order. The Nth element is
 * matched with the next entry whose level equals the element's level; an element whose level
 * differs from the next entry's (a heading the source scan does not see, such as one inside a
 * blockquote) is skipped and left without an id.
 * @param container The element holding the rendered note body.
 * @param entries The headings from `extractHeadings`, in document order.
 * @example
 * ```ts
 * assignHeadingIds(bodyElement, extractHeadings(source));
 * ```
 */
export function assignHeadingIds(container: Element, entries: readonly TocEntry[]): void {
  let next = 0;
  for (const el of container.querySelectorAll("h1, h2, h3, h4, h5, h6")) {
    if (next >= entries.length) break;
    const entry = entries[next];
    if (Number(el.tagName.slice(1)) !== entry.level) continue;
    el.id = entry.slug;
    next += 1;
  }
}

/**
 * Finds the element in `container` whose id is `slug`. The lookup compares ids directly, so it
 * is scoped to the container and never reaches a same-named id in another open sheet.
 * @param container The element holding the rendered note body.
 * @param slug The anchor id.
 * @returns The element, or `null` when none carries the id.
 * @example
 * ```ts
 * findAnchor(bodyElement, "intro")?.scrollIntoView();
 * ```
 */
export function findAnchor(container: Element, slug: string): Element | null {
  for (const el of container.querySelectorAll("[id]")) {
    if (el.id === slug) return el;
  }
  return null;
}
