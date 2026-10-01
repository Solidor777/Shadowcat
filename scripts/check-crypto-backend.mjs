// Fails when a second TLS crypto backend enters the dependency graph. The server pins `ring` as
// its one rustls provider (`tls::ring_provider`). `FORBIDDEN_CRATES` is the ONE list of what is
// forbidden (docs and comments elsewhere point at this gate instead of repeating it). Three
// families:
//   - `aws-lc-*`: a dependency enabled rustls's default `aws_lc_rs` feature, which adds a C/asm
//     build requirement (a C compiler, and NASM on some Windows targets), roughly doubles the
//     crypto footprint, and makes `rustls::ClientConfig::builder()` ambiguous at runtime (two
//     installed providers panic);
//   - `openssl`, `openssl-sys`, `native-tls`: the platform TLS stack, which needs a system OpenSSL
//     on Linux and breaks the single self-contained executable on every target (a dependency
//     enabled reqwest's default `native-tls`);
//   - `rustls-platform-verifier`: certificate verification through the operating system, which
//     reintroduces a per-platform code path (and on Android a JNI dependency) that the design
//     excludes: the server only ever verifies with the bundled roots its ACME client needs.
// `Cargo.lock` lists every package resolved for every target, so this check is stricter than one
// host's `cargo tree` and needs no toolchain, which keeps it runnable on any runner.

import { readFileSync } from "node:fs";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

/** Crate names whose presence in the lockfile means a second TLS backend is in the graph. */
export const FORBIDDEN_CRATES = [
  "aws-lc-rs",
  "aws-lc-sys",
  "aws-lc-fips-sys",
  "openssl",
  "openssl-sys",
  "native-tls",
  "rustls-platform-verifier",
];

/**
 * Package names in a `Cargo.lock` body that appear in `forbidden`.
 * @param {string} lockText The lockfile contents.
 * @param {string[]} [forbidden] Crate names to reject.
 * @returns {string[]} The forbidden names present, sorted and de-duplicated.
 */
export function findForbiddenCrates(lockText, forbidden = FORBIDDEN_CRATES) {
  const present = new Set();
  for (const m of lockText.matchAll(/^name = "([^"]+)"$/gm)) {
    if (forbidden.includes(m[1])) present.add(m[1]);
  }
  return [...present].sort();
}

if (isDirectEntry(import.meta.url)) {
  const found = findForbiddenCrates(readFileSync("Cargo.lock", "utf8"));
  if (found.length > 0) {
    console.error(
      `crypto backend check: ${found.join(", ")} present in Cargo.lock. The server's only rustls ` +
        "provider is ring; find the dependency pulling the crate in " +
        "(`cargo tree -i <crate> -e all`) and disable the feature that enables it " +
        "(rustls default features for aws-lc, reqwest default features for native-tls, the " +
        "`rustls-platform-verifier` feature of whichever client crate enabled it).",
    );
    process.exit(1);
  }
  console.log(`crypto backend check: none of ${FORBIDDEN_CRATES.join(", ")} in Cargo.lock`);
}
