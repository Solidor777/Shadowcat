// Composes the final dist-docs/ site: VitePress portal at the root, TypeDoc under
// api/ts/, rustdoc under api/rust/, then link-checks the PORTAL pages (generated
// references guarantee their own internal integrity; portal links INTO them are
// validated because the copied files are on disk by check time).
// Cross-platform invariant: node:path/node:fs only — no shell, no separators.
import { cpSync, existsSync, readdirSync, readFileSync, statSync, mkdirSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

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

/** Recursively list .html files under dir, skipping the given top-level subtrees. */
export function htmlFilesUnder(dir, skipSubtrees = []) {
  const out = [];
  const skip = new Set(skipSubtrees.map((s) => resolve(dir, s)));
  const walk = (d) => {
    if (skip.has(resolve(d))) return;
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(".html")) out.push(p);
    }
  };
  walk(dir);
  return out;
}

/** Copy portal/ts/rust trees into out (portal at root, refs under api/). */
export function assemble({ portal, ts, rust, out }) {
  mkdirSync(out, { recursive: true });
  cpSync(portal, out, { recursive: true });
  cpSync(ts, join(out, "api", "ts"), { recursive: true });
  cpSync(rust, join(out, "api", "rust"), { recursive: true });
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
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
  // Portal pages only; api/ subtrees are excluded as link SOURCES, present as targets.
  const portalPages = htmlFilesUnder(paths.out, [join("api", "ts"), join("api", "rust")]);
  const broken = checkLinks(paths.out, portalPages);
  if (broken.length > 0) {
    for (const b of broken) console.error(`dead link: ${b.source} -> ${b.target}`);
    process.exit(1);
  }
  console.log(`dist-docs assembled: ${portalPages.length} portal pages, links OK (root: ${paths.out}${sep})`);
}
