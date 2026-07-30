# Shadowcat Documentation System — Design

**Date:** 2026-07-30
**Status:** Approved design, pre-implementation
**Scope decision trail:** full-repo API reference (every function, all packages, server
internals included), examples on every function, static HTML site opened from disk,
VitePress portal, tutorial guides with in-repo CI-built worked examples, full CI
enforcement of doc coverage.

## Goal

A locally-browsable HTML documentation site for Shadowcat containing:

1. **API reference** — descriptions, parameter docs, and usage examples for every
   function in the repository: the Rust server crate (public and private items) and
   every TypeScript workspace package.
2. **Guides** — start-to-finish tutorials for (a) hosting a local Shadowcat server,
   (b) creating a module, and (c) creating a system.

The doc comments in source are the durable asset; every generator in the pipeline is
replaceable without losing them.

## Non-goals

- Publishing/hosting the site online (the output is a local static site; hosting it
  later is trivial but out of scope).
- Auto-generated reference pages for `.svelte` component APIs (no mature Svelte 5
  runes doc generator exists — covered instead by hand-written per-module pages; see
  §2 "Svelte gap").
- Embedding docs into the server binary (`dist-docs/` stays out of `rust-embed`).
- Documenting `src/types/generated` by hand — ts-rs output inherits doc comments from
  the Rust source types; the Rust side is where those get written.

## 1. Site architecture

A new VitePress project at `docs/site/` builds the portal. The complete site is
assembled into `dist-docs/` at the repo root (git-ignored):

```
dist-docs/
├── index.html            landing page: Guides / TS API / Rust API / Protocol
├── guides/
│   ├── hosting.html
│   ├── creating-a-module.html
│   └── creating-a-system.html
├── protocol.html         wire-protocol overview, links into api/ts generated types
├── modules/              hand-written per-module pages (one per src/modules/*)
├── api/ts/               TypeDoc output — all workspace packages, one merged site
└── api/rust/             rustdoc output — --document-private-items
```

- Guide/portal sources are Markdown under `docs/site/` (versioned with the repo).
- Root `package.json` scripts:
  - `pnpm docs:build` — runs TypeDoc → `cargo doc` → VitePress build → assembly
    into `dist-docs/`.
  - `pnpm docs:dev` — VitePress dev server for authoring guides/portal pages.
- Assembly is a Node script in `scripts/` (copy generated outputs into the VitePress
  dist, then run the link check). Cross-platform: `node:path`/`node:fs` only, no
  shell-isms, no hardcoded separators.
- Viewing: `pnpm docs:serve` (a small in-repo static server script). VitePress
  output uses absolute asset paths, so the portal does not render from `file://` —
  the site needs any static file server; `docs:serve` ships one so there is no
  extra install.
- `dist-docs/` and `docs/site/.vitepress/cache` are git-ignored.

### New dev-dependencies (root `package.json`)

`vitepress`, `typedoc`, `eslint-plugin-jsdoc`. No runtime dependencies. No server
code changes beyond doc comments and lint attributes.

## 2. API reference generation

### Rust (`src/server`, crate `shadowcat`)

- Generation: `cargo doc --document-private-items --no-deps` → `target/doc`, copied
  to `dist-docs/api/rust/` by the assembly script.
- The crate has both `lib.rs` and `main.rs`; items reachable from the lib target are
  what rustdoc documents and doctests run against. `main.rs` stays a thin entry.
- Coverage enforcement (constraint: the existing CI clippy step runs
  `-D warnings`, so warn-tier lint *attributes* would go red immediately):
  - Phase 1: an **informational CI step** runs
    `cargo clippy -- -W missing-docs -W clippy::missing-docs-in-private-items`
    (no `-D`; exits 0, reports counts). No crate attributes yet.
  - Sweep phases: each completed module adds `#![deny(missing_docs)]` +
    `#![deny(clippy::missing_docs_in_private_items)]` module-scoped attributes —
    these fail the normal clippy step on regression.
  - Final phase: crate-root deny attributes replace the per-module ones.
- Examples are `/// # Examples` doctest blocks — compiled and run by `cargo test`.
  Examples needing live infrastructure (DB pool, Room, sockets) use ` ```no_run `
  (compiled, not executed) rather than pseudo-code. ` ```ignore ` is banned.
- Example **presence** (stable Rust has no missing-example lint): a dedicated CI
  step runs rustdoc with the nightly-only `rustdoc::missing_doc_code_examples` lint
  (pinned nightly toolchain, used for this check only — the shipped binary stays on
  stable). It follows the same warn→deny ratchet as the coverage lints. If the
  nightly lint regresses, the step degrades to non-blocking with a logged warning
  until repaired — coverage/doctest gates above are unaffected.

### TypeScript (all pnpm workspace packages)

- Generation: a single root TypeDoc config using the `packages` entry-point strategy
  over the workspace (`src/types`, `src/client/*`, `src/modules/*`, plus
  `examples/*` once added) → one cross-linked HTML site at `dist-docs/api/ts/`.
- Coverage enforcement, two layers:
  - **TypeDoc validation** — `requiredToBeDocumented` covering functions, methods,
    classes, interfaces, type aliases, enums, variables; `--treatValidationWarningsAsErrors`
    in CI. Covers everything TypeDoc renders (exported symbols).
  - **ESLint** — `eslint-plugin-jsdoc` `require-jsdoc` + `require-param` +
    `require-description` for **all** functions including non-exported helpers,
    which TypeDoc cannot see. The root ESLint config currently ignores
    `**/*.svelte` entirely; Phase 1 adds `svelte-eslint-parser` so the jsdoc rules
    reach `.svelte` script blocks. Fallback if the jsdoc plugin proves incompatible
    with the svelte parser: `.svelte` doc coverage is review-enforced during the
    sweep phases instead. The jsdoc rules live in a separate docs ESLint config
    (`eslint.docs.config.js`, run as `pnpm lint:docs`) until the final ratchet
    phase merges them into the main config — `pnpm lint` stays warning-free
    meanwhile.
- Every function's doc comment carries an `@example` fenced ` ```ts ` block
  (enforced by `jsdoc/require-example`; see §5 for its staleness gate).
- Exclusions are explicit: any file TypeDoc/ESLint must skip (e.g. generated
  `src/types/generated`, config files, test files for `require-example`) is listed
  in config with a comment saying why. No silent skips.

### Wire protocol

The ts-rs generated types (`src/types/generated`) appear in the TS reference with
doc comments inherited from their Rust definitions. A hand-written `protocol.md`
portal page gives the overview — connection lifecycle, auth, sequence numbers/resync,
frame catalog — with each frame name linking to its generated type page.

### Svelte gap (explicit constraint)

`.svelte` component APIs (props/snippets/events) are not auto-rendered. Mitigation:

- Doc comments on every `$props()` declaration and script-block function (ESLint-
  enforced via the svelte processor where parseable).
- A hand-written portal page per first-party module under `modules/` — purpose,
  contributions declared, components provided, contracts consumed/provided,
  settings — kept to the same fixed shape per page.

## 3. Guides

Three start-to-finish tutorials (VitePress Markdown, `docs/site/guides/`):

1. **Hosting a local Shadowcat server** — obtain/build the binary (three OSes),
   config layering (CLI flag > `SHADOWCAT_*` env > TOML > default, per
   `src/server/src/config.rs`), first-run admin/world setup, inviting users
   (admin-created accounts + world invite/accept), backup/restore
   (`src/server/src/backup.rs` VACUUM-INTO), LAN access and reverse-proxy notes,
   per-OS notes (paths, firewalls).
2. **Creating a module** — supersedes and absorbs `docs/design/module-authoring.md`
   (that file is replaced by a pointer to the built guide source): scaffolding,
   `module.json` manifest (engines gate, folder-name identity, entry, requirements),
   Vite lib build with engine externals, writing a real contribution
   (the worked example: an initiative-tracker panel using the `shadowcat.panel`
   contract), installing into `modules_dir`, per-world enablement, dev loop.
3. **Creating a system** — build a minimal dice-based system: engine-typed docs vs
   opaque `system` band, templates for character/item doc types, a sheet via the
   `shadowcat.sheet:<doc_type>` contract, `@shadowcat/formula` for derived stats,
   dice/chat integration; Nightfox cited as the full-scale reference implementation.

### Worked examples (`examples/` at repo root)

- `examples/module-initiative-tracker/` — the module guide's complete code.
- `examples/system-minimal/` — the system guide's complete code.
- Both are pnpm workspace members: `pnpm -r build/test/typecheck` covers them in CI,
  so the guide code cannot rot. Both follow the external-module layout (own
  `module.json`, Vite lib config with externals) so they double as copyable
  scaffolds.
- Guides never paste code: VitePress code-import snippets include regions of the
  real example sources, so the rendered guide always shows the CI-built code.

## 4. Examples staleness gates

| Surface | Example form | Gate |
|---|---|---|
| Rust | `/// # Examples` doctest | `cargo test` runs it (or compiles it, `no_run`) |
| TS | `@example` ` ```ts ` block | `scripts/extract-ts-examples.mjs` extracts every block into a generated scratch package and typechecks it in CI (compile-checked, not executed) |
| Guides | code-import from `examples/` | `examples/*` are workspace members built/tested by CI |
| Portal links | internal links | link-check in the assembly script + VitePress dead-link detection fails the build |

`extract-ts-examples.mjs` details: scans workspace sources for `@example` blocks,
emits one `.ts` file per block into a scratch dir under the docs build output
(never committed), with an ambient import context matching the source package;
`tsc --noEmit` over the scratch package. Failure lists source file + line of the
offending example.

## 5. CI integration

A `docs` job added to the existing CI:

- Runs on one OS (ubuntu) for the site build: TypeDoc + `cargo doc` + VitePress +
  assembly + link check. Artifacts the `dist-docs/` output.
- The enforcement lints run wherever their host job already runs: clippy/rustdoc
  lints in the three-OS Rust matrix jobs, ESLint jsdoc rules in the existing lint
  job, TS example extraction + typecheck in the client test job.
- Doctests already run under `cargo test` in the matrix.

## 6. Phasing (campaign decomposition)

This is multiple implementation plans, sequenced; enforcement ratchets per-area so
coverage never regresses once a sweep lands.

- **Phase 1 — Infrastructure + guides:** VitePress portal, TypeDoc + rustdoc wiring,
  assembly + link-check scripts, example-extraction script, CI docs job, all lints
  landed at **warn**, the three guides, the two worked examples, per-module portal
  pages, protocol page. Deliverable: `pnpm docs:build` produces the full site with
  whatever doc comments exist.
- **Phases 2–N — Doc-comment sweeps,** one subsystem per plan:
  - Server: `data/`, `ws/`, `http/` + `auth/`, `scene/`, `chat/` + `dice/`,
    remaining (`config`/`db`/`backup`/`modules`/bootstrap).
  - Client: `core`, `render`, `ui-kit` + `shell`, `formula`.
  - Modules: grouped into 3–4 plans (~5–7 packages each).
  - Each sweep documents every symbol + example in its area and flips that area's
    lint scope to deny (Rust: per-module `#![deny(...)]` attributes; TS: per-package
    ESLint override severity).
- **Final phase:** repo-wide deny (crate-root attributes + root ESLint severity),
  remove the per-area overrides, confirm three-OS CI green.

Milestone placement in `docs/PLAN.md` is decided at writing-plans time (this is a
documentation campaign parallel to feature work; it does not block D-beta/Phase-E).

## 7. Error handling

The docs build fails loudly, never silently:

- Missing doc comment → lint error (per ratchet phase).
- Broken Rust example → doctest failure in `cargo test`.
- Broken TS example → extraction typecheck failure with source location.
- Dead internal link → link-check/VitePress failure of the docs job.
- Unresolvable TypeDoc reference → validation error.
- Every exclusion is an explicit, commented config entry.

## 8. Testing

- The assembly and extraction scripts get Vitest coverage (following the existing
  `scripts/check-svelte-runtime-entries.test.mjs` pattern): fixture packages with
  good/bad examples, link fixtures with a known dead link, assembly into a temp dir.
- The worked examples carry their own unit tests (run by `pnpm -r test`) so the
  guide code is exercised, not just compiled.
- CI is the integration test: the docs job building `dist-docs/` end-to-end.

## Open items intentionally deferred to writing-plans

- Exact TypeDoc theme/plugin choices (default theme unless a need appears).
- Initiative-tracker example's exact feature set (kept minimal; panel + document
  read/write + one contribution contract).
- Per-module portal page template wording.
