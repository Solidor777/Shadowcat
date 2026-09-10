import { test, expect } from "vitest";
import { resolve } from "node:path";
import { cargoDocInvocation, MODES } from "./cargo-doc-strict.mjs";

const REPO = resolve("/", "repo");
const MANIFEST = resolve(REPO, "src", "server", "Cargo.toml");

test("warnings mode: stable toolchain, -D warnings, shared target dir", () => {
  const { args, env } = cargoDocInvocation("warnings", REPO);
  expect(args).toEqual(["doc", "--manifest-path", MANIFEST, "--document-private-items", "--no-deps"]);
  expect(env).toEqual({ RUSTDOCFLAGS: "-D warnings" });
});

test("examples mode: nightly toolchain, the missing-example lint denied, its own target dir", () => {
  const { args, env } = cargoDocInvocation("examples", REPO);
  expect(args[0]).toBe("+nightly");
  expect(args).toContain("--document-private-items");
  expect(args).toContain("--no-deps");
  expect(args.slice(args.indexOf("--target-dir"))).toEqual([
    "--target-dir",
    resolve(REPO, "target", "nightly-doc"),
  ]);
  expect(env).toEqual({ RUSTDOCFLAGS: "-D rustdoc::missing_doc_code_examples" });
});

// The environment is what distinguishes the two modes from a bare `cargo doc`: an invocation
// that dropped it would run green on the exact defect each mode exists to catch.
test("every mode sets RUSTDOCFLAGS to a -D flag; no mode is a bare cargo doc", () => {
  for (const mode of Object.keys(MODES)) {
    const { env } = cargoDocInvocation(mode, REPO);
    expect(env.RUSTDOCFLAGS).toMatch(/^-D /);
  }
});

test("an unknown or missing mode is rejected by name", () => {
  expect(() => cargoDocInvocation("strict", REPO)).toThrow(/unknown mode: strict/);
  expect(() => cargoDocInvocation(undefined, REPO)).toThrow(/warnings, examples/);
});
