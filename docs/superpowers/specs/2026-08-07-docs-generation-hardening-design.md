# Documentation-generation hardening — design

Date: 2026-08-07
Branch of origin: `docs-sweep13-property-coverage`

## Problem

The documentation gates that run on *source* are green: `pnpm lint:docs`,
`pnpm lint:props`, and `pnpm lint:comments` all pass. The gates that run on the
*generators* do not exist.

`pnpm docs:build` completes successfully while emitting:

- **TypeDoc: 0 errors, 1299 warnings**
- **rustdoc: 11 warnings**

Neither fails the build, because `typedoc.json` sets
`treatValidationWarningsAsErrors: false` and the CI `cargo doc` step sets no
`RUSTDOCFLAGS`. A third setting, `skipErrorChecking: true`, disables TypeDoc's
type-checking altogether.

This contradicts the project's stated position that every documentation check is
a gate and none is warn-tier: a reported-but-passing violation is
indistinguishable to a later reader from output that was checked and passed.

Two adjacent problems ship in the same change because they share the same
pipeline:

- Documentation regeneration is not part of any build. It runs only when
  someone types `pnpm docs:build`, so the site is stale by default.
- The assembled site cannot be opened by double-clicking `dist-docs/index.html`.
  Reading the docs requires starting a server from a console.

## Goals

1. TypeDoc and rustdoc emit zero warnings, and warnings thereafter fail CI.
2. Documentation regeneration is part of the full build, and the published
   documentation artifact is rebuilt from scratch on every CI run.
3. `dist-docs/index.html` opens by double-click on Windows, macOS, and Linux.
4. Every `shadowcat-codebase-*` skill points into the generated documentation.
5. The work merges to `main` and CI is green.

## Non-goals

- Rewriting the documentation site's content, navigation, or theme.
- Changing what `pnpm build` does or how long it takes.
- Client-side search over `file://`. Browsers block module scripts from
  `file://`, so search remains a served-mode feature. This is accepted, not
  worked around.

## Decisions taken

| Question | Decision |
|---|---|
| Where docs regeneration hooks into the build | Split fast vs full: `pnpm build` stays client-only; a new `build:all` runs client + docs and is what CI invokes |
| Whether the distributable ships documentation | No, by owner ruling. `target/package/` carries the binary and icon only; documentation is published as a separate CI artifact |
| How the index becomes clickable | Rewrite the portal's absolute paths to depth-relative in the assembly step; keep `docs:serve` for full fidelity |
| Whether generator warnings fail CI | Yes |
| `skipErrorChecking` | Set to `false` (measured: 0 errors, so no cost) |
| Form of the skill documentation pointers | Site-relative URL paths (`/api/rust/shadowcat/scene/`), not `dist-docs/` filesystem paths |
| ts-rs synthesized discriminants | Exempt, with explicit owner sign-off (see *The one exemption*) |

## Design

### 1. TypeDoc warnings to zero

The 1299 warning lines collapse to 452 distinct entries in four classes — 401
"not documented" (classes A and B), 43 unbound `@param` names (class C), and 8
link failures (class D). Each class has a different cause and a different fix.

The gap between 1299 lines and 452 entries is almost entirely class A: one
undocumented leaf is reported once per path by which it is reachable, so the
line count overstates the work by roughly a factor of three. Progress must be
measured in distinct entries; the raw warning count will fall in jumps that do
not correspond to effort.

#### A. Members of anonymous inferred shapes — ~384 distinct entries

TypeDoc expands a Zod schema's *inferred output type* and requires
documentation on every leaf property, at every path by which that leaf is
reachable. One undocumented leaf therefore produces many warning lines. JSDoc
cannot be attached to an inferred member, so these cannot be documented where
they are flagged.

The fix is already demonstrated in the codebase. `DocumentSchema` is annotated:

```ts
export const DocumentSchema: z.ZodType<WireDocument> = z.lazy(() => z.object({ ... }));
```

and produces exactly **one** warning — its own missing doc comment. The
unannotated `ChatMessageEngineSchema` produces **188**. The explicit annotation
stops the expansion, because TypeDoc documents the named type instead of the
anonymous inferred one.

So: every exported schema constant that TypeDoc expands gains a documented named
type plus an explicit `z.ZodType<T>` annotation and a doc comment on the
constant itself.

**The named types do not already exist.** Today they are derived *from* the
schemas — `export type WireAudience = z.infer<typeof AudienceSchema>` — so
annotating a schema with its own inferred type is circular and does not
compile. Each affected schema needs a hand-written type declaration whose
members carry the documentation, in the shape `WireDocument` already uses. This
is the cost of the class: roughly ten type declarations, not ten annotations.

Hand-writing both a schema and its type does **not** create a forked decision.
`z.ZodType<T>` is checked by the compiler: a schema that stops producing exactly
`T` fails to build. The agreement is structural, which is the property the
architecture asks for, and `DocumentSchema` / `WireDocument` is the established
in-repo precedent.

**The work cascades, so the entry counts overstate it.** Functions whose return
type is an inferred schema output are re-reported against that anonymous shape:
`parseServerMsg` (79 entries), `parseMessageEngine` (47), and `isKnownSegment`
(33) carry no warnings of their own. Naming `ServerMsgSchema`,
`ChatMessageEngineSchema`, and `ChatSegmentSchema` clears all three. Likewise
`computeRevert` and `planToUpdate` (8 each) follow `OperationSchema`, and
`SearchPage` follows `SearchHitSchema`.

Root schemas requiring a hand-written documented type: `ServerMsgSchema`,
`ChatMessageEngineSchema`, `ChatSegmentSchema`, `RollOutcomeSchema`,
`CommandSchema`, `DieRecordSchema`, `OperationSchema`, `SearchHitSchema`,
`CapabilityRequirementSchema`, `ManifestSchema`. `MessageKindSchema` and
`DocumentSchema` need only a doc comment on the constant.

This is worth doing independently of the warning count. The architecture names
*forked decisions* — two paths documented to agree, which later disagree on an
input nobody checked — as the defect class this codebase produces most. An
annotated schema cannot drift from its wire type without a compile error, which
converts a documented agreement into a structural one.

Inline object shapes on function signatures (`computeRevert`, `planToUpdate`,
`StampOpts.permissions`, `parseServerMsg`) are extracted into named documented
types by the same reasoning.

**Implementation risk, measured and closed.** `z.ZodType<T>` erases the schema's
own surface, so a call site using `.shape`, `.extend()`, `.partial()`, `.pick()`,
`.omit()`, or `.merge()` on an annotated constant would stop compiling. A
repo-wide search across `src`, `examples`, and `scripts` finds **zero** such call
sites, so the annotation is safe everywhere and no `satisfies`-based fallback is
needed. If a future change introduces one, the build fails loudly rather than
silently, which is the correct failure mode.

#### B. Undocumented named symbols — 17 distinct entries

Ordinary missing doc comments: `silentLogger`, the four `formula` limit
constants (`MAX_FORMULA_LENGTH`, `MAX_AST_NODES`, `MAX_PARSE_DEPTH`,
`MAX_GRAPH_VISITS`), `MESSAGE_DOC_TYPE`, `CHANNEL_REGISTRY_DOC_TYPE`, the
schema constants covered in A, and the shell's default export. Each gets a doc
comment stating the constraint it encodes, not a restatement of its name.

#### C. `@param` names not bound to a signature — 43 distinct entries

Comments document `@param opts.channel` where the signature takes an
undestructured `opts` object, so TypeDoc cannot bind the dotted name and the
documentation renders nowhere.

Fix by naming the options type and documenting its fields on that type, leaving
`@param opts` as a single line. The field documentation then appears in the
rendered API output, which is where a reader looks for it. This overlaps with A
and should be done in the same pass over each file.

Affected signatures span `WsClient` (`search`, `subscribeScene`,
`subscribeSearch`, `sendChatMessage`, `moveRequest`, `pathfind`), `ChatApi.send`,
`AppContext.subscribeScene`, `ServiceRegistry.provide`,
`ContributionRegistry.contribute`, `ModuleRegistry.unload`, `loadModules`,
`createUser`, `createPixiBackend`, `TokenView.setAnimationConfig`,
`RenderEngine.setAnimation`, and the `WsClientHandlers` / `AssetResolver`
callback shapes.

#### D. Unresolved or unexported link targets — 8 distinct entries

- `Grid.snap` links to `axialRound`, `axialToPixel`, `hexLines`; `Lighting`
  links to `LIGHTING_FADE_MS`; `RenderEngine.compositor` links to
  `applyVisionSweep`. Each target is resolved but not exported. Export the
  target where it is genuinely part of the API; otherwise restate the reference
  as plain backticked prose.
- `RenderEngine.compositor` links to `renderVisibility`, which resolves to
  nothing. Find the real symbol or drop the link.
- `PanelsBridge.open` links to `PanelsBridge.#warnOnce`, a private field. A link
  to a private member is unresolvable by construction; rewrite the sentence.

`externalSymbolLinkMappings` is **not** used for any of these. TypeDoc suggests
it in the warning text, but mapping a symbol to `"#"` silences the diagnostic
while leaving the reader with a link that goes nowhere.

### 2. rustdoc warnings to zero

All 11 are punctuation parsed as intra-doc link syntax, not missing content:

- 7 occurrences of interval notation — `[0,1]` in `data/engine/scene.rs` and
  `scene/lighting.rs`, `path[0]` in `scene/move_exec.rs`.
- 2 unclosed HTML tags — `<explicit token>` in `config.rs`, `<world_id>/<uuid>`
  in `data/asset.rs`.
- 1 unresolved link to `SECRET_BYTES`.
- 1 public doc item linking the private `SESSION_SWEEP_PERIOD`, which resolves
  only because `--document-private-items` is passed and would break without it.

All are fixed by backticking the offending span. Backticks are chosen over
backslash escapes because the project's convention is to cite symbols and
values in backticks anyway, so the fix and the house style coincide.

### 3. Gates

- `typedoc.json`: `treatValidationWarningsAsErrors: true`, and
  `skipErrorChecking: false`. The second was measured before being proposed:
  running TypeDoc with type-checking enabled reports 0 errors, so enabling it
  costs nothing and closes a real hole — TypeDoc currently documents code it
  never type-checks.
- The CI `cargo doc` step gains `RUSTDOCFLAGS: -D warnings`.
- The nightly `rustdoc::missing_doc_code_examples` step already denies and is
  unchanged.

### 4. Build integration

One command chain, reachable under an intent-revealing name, with no duplicated
command strings:

```
docs:generate   typedoc + cargo doc + vitepress portal + assemble
build:all       pnpm build && pnpm docs:generate
docs:build      delegates to build:all
build           unchanged — client only
```

`docs:build` is retained as a delegating alias so CI, documentation, and habit
keep working.

`scripts/package.sh` is unchanged and invokes no build of its own. It requires
`target/release/shadowcat` to exist already, then copies that binary and the
application icon into `target/package/`; it never copies `dist-docs/`, so the
distributable carries no documentation to go stale. Documentation ships as its
own CI artifact, rebuilt from scratch by the docs job on every run, which is
what keeps it current.

The client build must remain first in the chain: `rust-embed` validates `dist/`
at compile time, so `cargo doc` fails without it.

### 5. Clickable index

rustdoc and TypeDoc output already use relative paths and already open over
`file://`. Only the VitePress portal blocks it, emitting root-absolute
references (`/assets/style.css`, `/guides/hosting.html`, `/modules/`). VitePress
has no relative-base option, so the rewrite happens after it runs.

A new pass in `scripts/assemble-docs.mjs`, between `assemble()` and the existing
link check:

- Walk portal HTML, skipping the `api/ts` and `api/rust` subtrees.
- Rewrite each root-absolute `href`/`src` to a path relative to the containing
  file's depth under `dist-docs/`.
- Rewrite root-absolute `url(...)` references in portal CSS, which is how the
  bundled fonts are reached.
- Expand directory-style targets to an explicit `index.html`
  (`/modules/` becomes `../modules/index.html`). This is required: `file://`
  does not resolve a bare directory to its index, and this is the single detail
  most likely to be missed, because it works over HTTP either way.
- Leave scheme-prefixed, protocol-relative, and fragment-only references alone.

Absolute paths embedded in the VitePress runtime JavaScript are **not**
rewritten. Over `file://` the browser refuses to load module scripts at all, so
those paths are never dereferenced; over HTTP the runtime behaves exactly as it
does today.

Verification is mechanical rather than visual:

- Assert that zero root-absolute local references remain in portal HTML.
- Re-run the existing `checkLinks`, which resolves relative links against each
  file's own directory — the same resolution a browser performs over `file://`.

**Claims this falsifies.** Widening what the site supports makes three existing
statements false. They are corrected in the same change, because none of them
appears in the diff of the code that falsifies them:

- `serve-docs.mjs`'s header comment, which states that absolute asset paths do
  not render from `file://`.
- The `shadowcat-codebase-core` skill's note that `pnpm docs:serve` is required
  because `file://` is unsupported.
- The documentation index's *reading these docs locally* section.

### 6. Skill documentation pointers

All 16 skills — `shadowcat-codebase-core` plus the 15 subsystem skills — gain
documentation pointers in their Pointers section, as site-relative URL paths:

```
- Generated API: `/api/rust/shadowcat/scene/`, `/api/ts/modules/_shadowcat_render.html`
- Guide: `/guides/creating-a-module`
```

Site-relative rather than `dist-docs/` filesystem paths, because the same
citation then resolves whether the reader double-clicks the index or serves it,
and because `dist-docs/` is git-ignored build output that does not exist on a
fresh clone.

Skills stay orientation-and-index: they point into the documentation and never
restate it. The skill diffs go through the reviewed skill-update gate —
`shadowcat-spec-reviewer` confirms each diff accurately captures the change,
with no omission, drift, or broken pointer.

### 7. Merge and push

`docs-sweep13-property-coverage` merges to `main`, pushes, and CI is watched to
green.

## The one exemption

Eight warnings name a discriminant on a ts-rs generated union:

```
AnimatedSource.__type.type    Operation.__type.op      Scope.__type.kind
Audience.__type.kind          RenderVisual.__type.kind ServerMsg.__type.type
ClientMsg.__type.type         TokenVisual.__type.kind
```

These cannot be documented at the source. ts-rs *does* propagate Rust doc
comments — the generated `Scope` carries the documentation of `pack` and
`world_id` verbatim. What it drops is the doc comment on the *variant*, and the
key it flags (`"kind": "compendium"`) is synthesized by serde's `tag`
attribute. There is no declaration anywhere to attach a doc comment to.

The owner ruled these exempt, with explicit sign-off, over the two alternatives:
post-processing the generated bindings to re-inject variant documentation, which
creates a second generator that must stay in agreement with ts-rs — the exact
forked-decision pattern the architecture names as its most common defect — and
patching ts-rs upstream, which blocks this work on an external release.

**Implemented narrower than approved.** The sign-off was for exempting
`src/types/generated/**`. The implementation uses TypeDoc's
`intentionallyNotDocumented`, which takes a list of **full reflection names**,
and enumerates exactly the eight. A path glob would silently absorb any future
generated discriminant; an enumerated list fails the gate until someone adds the
name deliberately. This can only shrink the exemption, never widen it, so it
needs no further approval — but it is recorded here because the implemented form
differs from the approved form.

The exemption prints its active count. `docs:generate` reports the number of
`intentionallyNotDocumented` entries in effect, because an uncounted exemption
is a backdoor and a silent one is indistinguishable from a rule that does not
apply.

## Testing

- **`scripts/assemble-docs.test.mjs`** gains cases for the rewrite pass: depth
  0, 1, and 2 pages; directory-style targets expanded to `index.html`;
  scheme-prefixed, protocol-relative, and fragment-only references left
  untouched; CSS `url(...)` rewriting. Run by `pnpm test:scripts`.
- **The gates are the test for sections 1 through 3.** With warnings fatal,
  `pnpm docs:generate` exiting 0 is the evidence, and it cannot be satisfied by
  a partial fix.
- **A structural assertion** that no root-absolute local reference survives in
  portal HTML, run as part of assembly rather than as a separate check, so it
  cannot be skipped.
- **Full local verification before merge**: `pnpm -r test`, `pnpm -r typecheck`,
  `pnpm lint`, the four documentation gates, `pnpm test:scripts`, and from
  `src/server/`: `cargo test`, `cargo clippy`, `cargo fmt --check`.

## Risks

| Risk | Handling |
|---|---|
| `z.ZodType<T>` erases the schema surface and would break `.shape` / `.extend()` call sites | Measured: zero such call sites exist across `src`, `examples`, `scripts`. Risk closed, not mitigated |
| Hand-writing a type beside its schema lets the two drift | The compiler is the shared symbol: `z.ZodType<T>` fails to build if the schema stops producing `T` |
| The `file://` directory-link detail passes over HTTP and fails only on double-click | Covered by an explicit unit-test case, not by manual inspection |
| Warnings-as-errors makes an unrelated future change fail the docs job | Intended. It is the point of the change |
| Classes A and C touch the wire surface across `wire.ts` and `chat-docs.ts` | Wire types are mirrored by a parity guard against the Rust source; `pnpm -r test` and `pnpm -r typecheck` must both pass, not typecheck alone |

## Order of work

1. rustdoc warnings to zero, then `-D warnings` in CI. Smallest and fully
   independent.
2. TypeDoc classes B and D — named symbols and links. Independent of the schema
   work.
3. TypeDoc classes A and C — schema annotation and options types, file by file.
   The largest piece.
4. `intentionallyNotDocumented` enumeration and its printed count.
5. Flip `treatValidationWarningsAsErrors` and `skipErrorChecking`.
6. Script restructure — `docs:generate`, `build:all`, `docs:build` delegation.
7. The relative-path rewrite, its tests, and the three falsified claims.
8. Skill pointers, reviewed by `shadowcat-spec-reviewer`.
9. Merge, push, watch CI.

Steps 1 through 5 must land before 6, so that the build integration wires in a
chain that is already clean. Otherwise `build:all` starts life red.
