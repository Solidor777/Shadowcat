// Minimal static server for dist-docs/. Static content, styling, and ordinary link
// navigation already work by opening the assembled site directly over file://; this
// server exists for full fidelity, since anything driven by the site's runtime
// JavaScript — including client-side search, the appearance toggle, and the mobile
// nav panel — depends on a module script, and browsers refuse to load module scripts
// from file://. No dependencies.
import { createServer } from "node:http";
import { createReadStream, existsSync, statSync } from "node:fs";
import { extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const ROOT = resolve(fileURLToPath(import.meta.url), "..", "..", "dist-docs");
const PORT = Number(process.argv[2] ?? 4173);
const MIME = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript",
  ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml",
  ".png": "image/png", ".jpg": "image/jpeg", ".gif": "image/gif", ".ico": "image/x-icon",
  ".woff": "font/woff", ".woff2": "font/woff2", ".ttf": "font/ttf", ".map": "application/json",
  ".txt": "text/plain; charset=utf-8", ".wasm": "application/wasm",
};

if (!existsSync(ROOT)) {
  console.error(`dist-docs/ not found — run pnpm docs:build first`);
  process.exit(1);
}

createServer((req, res) => {
  // decodeURIComponent throws URIError on malformed %-sequences; a synchronous
  // throw in a request listener is an uncaught exception that kills the process.
  let urlPath;
  try {
    urlPath = decodeURIComponent((req.url ?? "/").split("?")[0]);
  } catch {
    res.writeHead(400);
    res.end();
    return;
  }
  // INVARIANT: resolved path stays inside ROOT (path-traversal guard). The
  // separator-bounded check also rejects sibling dirs sharing ROOT as a string
  // prefix (e.g. dist-docs vs dist-docs-other), which bare startsWith admits.
  const file0 = normalize(join(ROOT, urlPath));
  if (file0 !== ROOT && !file0.startsWith(ROOT + sep)) { res.writeHead(403); res.end(); return; }
  let file = file0;
  if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
  if (!existsSync(file)) { res.writeHead(404); res.end("404"); return; }
  res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
}).listen(PORT, () => console.log(`docs at http://localhost:${PORT}/`));
