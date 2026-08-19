// Composes the final dist-docs/ site: VitePress portal at the root, TypeDoc under
// api/ts/, rustdoc under api/rust/, then link-checks the PORTAL pages (generated
// references guarantee their own internal integrity; portal links INTO them are
// validated because the copied files are on disk by check time).
// Cross-platform invariant: node:path/node:fs only — no shell, no separators.
import { cpSync, existsSync, readdirSync, readFileSync, statSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

// Skips scheme-prefixed URLs, fragments, and protocol-relative (//host) URLs.
const SKIP_SCHEMES = /^(?:[a-z][a-z0-9+.-]*:|#|\/\/)/i;

/** Local link targets (href/src) in one HTML string; fragments/queries stripped. */
export function extractLocalLinks(html) {
  const out = [];
  for (const m of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const raw = m[1];
    if (SKIP_SCHEMES.test(raw)) continue;
    const clean = raw.split("#")[0].split("?")[0];
    if (clean !== "") out.push(decodeURIComponent(clean));
  }
  return out;
}

/** Broken links across htmlFiles, resolved against each file's directory.
 * A trailing-slash or extensionless-directory target resolves to its index.html. */
export function checkLinks(rootDir, htmlFiles) {
  const broken = [];
  for (const file of htmlFiles) {
    const html = readFileSync(file, "utf8");
    for (const link of extractLocalLinks(html)) {
      const base = link.startsWith("/") ? join(rootDir, link) : resolve(dirname(file), link);
      const target = existsSync(base) && statSync(base).isDirectory() ? join(base, "index.html") : base;
      if (!existsSync(target)) broken.push({ source: file, target });
    }
  }
  return broken;
}

/** Recursively list files with the given extension under dir, skipping the given
 * top-level subtrees. Shared walk backing htmlFilesUnder and cssFilesUnder. */
function filesUnder(dir, ext, skipSubtrees) {
  const out = [];
  const skip = new Set(skipSubtrees.map((s) => resolve(dir, s)));
  const walk = (d) => {
    if (skip.has(resolve(d))) return;
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(ext)) out.push(p);
    }
  };
  walk(dir);
  return out;
}

/** Recursively list .html files under dir, skipping the given top-level subtrees. */
export function htmlFilesUnder(dir, skipSubtrees = []) {
  return filesUnder(dir, ".html", skipSubtrees);
}

/** Recursively list .css files under dir, skipping the given top-level subtrees. */
export function cssFilesUnder(dir, skipSubtrees = []) {
  return filesUnder(dir, ".css", skipSubtrees);
}

/** Rewrite one root-absolute local link to a path relative to a page at `depth`
 * directories below the site root. Scheme-prefixed, protocol-relative, fragment-only
 * and already-relative links pass through untouched.
 * A directory target is expanded to its index.html: file:// does not resolve a bare
 * directory, unlike an HTTP server.
 * The fragment and query are split off before the trailing-slash test and reattached
 * after the path is rewritten, mirroring extractLocalLinks's split order, so a query
 * VALUE (which may itself end in a slash or carry a path) is never mistaken for the
 * link's own path. */
export function toRelativeHref(link, depth) {
  if (SKIP_SCHEMES.test(link) || !link.startsWith("/")) return link;
  const hash = link.indexOf("#");
  const frag = hash === -1 ? "" : link.slice(hash);
  const beforeFrag = hash === -1 ? link : link.slice(0, hash);
  const q = beforeFrag.indexOf("?");
  const query = q === -1 ? "" : beforeFrag.slice(q);
  let path = q === -1 ? beforeFrag : beforeFrag.slice(0, q);
  if (path.endsWith("/")) path += "index.html";
  const prefix = depth === 0 ? "./" : "../".repeat(depth);
  return prefix + path.slice(1) + query + frag;
}

/** Root-absolute value on ANY HTML attribute, either quote style — deliberately
 * broader than the href/src/double-quote-only pattern rewriteAbsolutePaths itself
 * rewrites, so a form the rewrite does not recognise (a single-quoted attribute, or
 * an attribute other than href/src carrying a local reference) still fails the
 * structural check below instead of shipping unrewritten. */
const ROOT_ABSOLUTE_ATTR = /[a-zA-Z_:][-\w:.]*\s*=\s*(?:"\/(?!\/)[^"]*"|'\/(?!\/)[^']*')/;

/** Root-absolute CSS url(...) reference, tolerant of whitespace around the quotes
 * and inside the parens — deliberately broader than the tight pattern
 * rewriteAbsolutePaths itself rewrites, matching what an unminified build (which
 * CSS syntax permits) would produce. */
const ROOT_ABSOLUTE_URL = /url\(\s*(?:"\s*\/(?!\/)[^"]*"|'\s*\/(?!\/)[^']*'|\/(?!\/)[^"')]*)\s*\)/;

/** True if a root-absolute local reference survives in a portal file — checks the
 * attribute predicate for HTML, the url() predicate for CSS, verifying the REWRITE'S
 * RESULT rather than merely echoing the rewrite's own recognition of what needed
 * changing. */
export function hasSurvivingAbsoluteRef(file, content) {
  return file.endsWith(".css") ? ROOT_ABSOLUTE_URL.test(content) : ROOT_ABSOLUTE_ATTR.test(content);
}

/** Rewrite root-absolute local refs in the given portal files to depth-relative
 * ones, so the assembled site resolves under file:// as well as over HTTP.
 * Returns the number of files changed. */
export function rewriteAbsolutePaths(rootDir, files) {
  let changed = 0;
  for (const file of files) {
    const depth = relative(rootDir, dirname(file)).split(sep).filter(Boolean).length;
    const before = readFileSync(file, "utf8");
    const after = file.endsWith(".css")
      ? before.replace(/url\(("|')?(\/[^"')]+)\1?\)/g, (m, q = "", p) => `url(${q}${toRelativeHref(p, depth)}${q})`)
      : before.replace(/(href|src)="([^"]+)"/g, (m, attr, link) => `${attr}="${toRelativeHref(link, depth)}"`);
    if (after !== before) {
      writeFileSync(file, after);
      changed += 1;
    }
  }
  return changed;
}

/** The portal files still carrying a root-absolute local reference once `rewriteAbsolutePaths`
 * has run. A non-empty result fails the docs build: such a reference resolves only when the
 * site is served from its own root, so the assembled tree would break under file://. */
export function survivingAbsoluteRefs(files) {
  return files.filter((f) => hasSurvivingAbsoluteRef(f, readFileSync(f, "utf8")));
}

/** Copy portal/ts/rust trees into out (portal at root, refs under api/).
 * `out` is regenerated wholesale from these three inputs on every call, so it is cleared first:
 * `cpSync` overwrites a destination file whose source still produces it, but never removes one
 * whose source no longer does, which would otherwise accumulate stale files across rebuilds. */
export function assemble({ portal, ts, rust, out }) {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  cpSync(portal, out, { recursive: true });
  cpSync(ts, join(out, "api", "ts"), { recursive: true });
  cpSync(rust, join(out, "api", "rust"), { recursive: true });
}

if (isDirectEntry(import.meta.url)) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const paths = {
    portal: join(repo, "docs", "site", ".vitepress", "dist"),
    ts: join(repo, ".docs-tmp", "api", "ts"),
    rust: join(repo, "target", "doc"),
    out: join(repo, "dist-docs"),
  };
  for (const [k, p] of Object.entries(paths)) {
    if (k !== "out" && !existsSync(p)) {
      console.error(`assemble-docs: missing input '${k}' at ${p} — run the full pnpm docs:build chain`);
      process.exit(1);
    }
  }
  assemble(paths);
  // Portal only; api/ subtrees are already relative (TypeDoc/rustdoc output) and untouched.
  const apiSubtrees = [join("api", "ts"), join("api", "rust")];
  const portalPages = htmlFilesUnder(paths.out, apiSubtrees);
  const portalStyles = cssFilesUnder(paths.out, apiSubtrees);
  rewriteAbsolutePaths(paths.out, [...portalPages, ...portalStyles]);
  const stillAbsolute = survivingAbsoluteRefs([...portalPages, ...portalStyles]);
  if (stillAbsolute.length > 0) {
    for (const f of stillAbsolute) console.error(`root-absolute reference survived rewrite: ${f}`);
    process.exit(1);
  }
  const broken = checkLinks(paths.out, portalPages);
  if (broken.length > 0) {
    for (const b of broken) console.error(`dead link: ${b.source} -> ${b.target}`);
    process.exit(1);
  }
  console.log(`dist-docs assembled: ${portalPages.length} portal pages, links OK (root: ${paths.out}${sep})`);
}
