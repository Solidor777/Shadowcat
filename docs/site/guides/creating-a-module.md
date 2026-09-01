# Creating a module

This tutorial builds a complete external Shadowcat module — an initiative-tracker
panel — from scaffold to a world where it runs. Every code sample on this page is
imported directly from `examples/module-initiative-tracker/` in the Shadowcat
repository, which CI builds and tests on every push: the code you read here is the
code that runs.

## What a module is

A Shadowcat module is a **client-only contribution package**: a JavaScript library
the browser loads into the running client, which registers UI and behavior through
contribution contracts. The server never runs module code — it only stores,
serves, and gates it (Shadowcat's server runs no third-party code, ever).

Two consequences to internalize before you start:

- **Modules are admin-trusted.** There is no sandbox. An installed module is
  client code at the same trust tier as the Shadowcat binary itself. Install
  modules you trust; ship modules worthy of that trust.
- **The server stays authoritative.** Your module *requests* changes (optimistic
  intents); the server validates, applies, and broadcasts. Nothing your module
  draws or writes locally bypasses server permissions.

## Scaffold

Copy `examples/module-initiative-tracker/` from the Shadowcat repository as your
starting layout:

| File | Role |
|---|---|
| `module.json` | The manifest: identity, engine-compat gate, contract declarations |
| `package.json` | npm package: deps on `@shadowcat/*`, build/test/typecheck scripts |
| `vite.config.ts` | Library build — one ES entry, engine packages external |
| `tsconfig.json` | Extends the repo base config; includes `.svelte` sources |
| `svelte.config.js` | `vitePreprocess()` — standard Svelte 5 preprocessing |
| `vitest.config.ts` / `vitest.setup.ts` | Unit tests in jsdom |
| `src/index.ts` | Module entry: manifest + `register(ctx)` |
| `src/InitiativePanel.svelte` | The contributed panel component |

## The manifest

One `module.json` sits at your module's install-folder root. The server serves it
byte-for-byte at `GET /api/modules`; the client validates it with its own Zod
schema.

<<< @/../../examples/module-initiative-tracker/module.json

Field by field:

- **`id`** — your module's declared identity. The client cross-checks it against
  the loaded entry's manifest, but the **install-folder name is the real key**:
  the server keys each world's enabled-module set on the folder name it controls,
  never on your declared `id`. Keep folder name and `id` identical.
- **`version`** — semver of your module.
- **`engines.shadowcat`** — a semver range checked against the running server's
  version at **enable time** (a GM toggles the module on) *and* **load time** (a
  client imports it). A community module without this field can never be enabled.
- **`dependencies`** — other module ids this module needs, with semver ranges.
- **`provides` / `requires`** — UI contract declarations. This module requires
  the panel-manager's `shadowcat.panel` contract and provides nothing of its own.
- **`requirements`** — declarative path-prefix → capability rules, unioned into
  the world's broadcast `capability_requirements` wherever the module is enabled.
  **Advisory to the client only**: the server never enforces third-party-declared
  requirements; real authority stays with the GM's world capability rules.
- **`entry`** *(optional, default `"index.js"`)* — overrides the built entry file
  name the server serves at `/modules/<folder-id>/<entry>`. Only declare it when
  your build emits something other than `index.js`.
- **`style`** *(optional)* — a single CSS file relative to your install folder
  (conventionally `"style.css"`). The server serves it from the module's static
  route and each client injects it as a `<link>` while your module is active,
  removing it again when the module is disabled or the world is left. Declare
  it whenever your build emits a stylesheet — an undeclared stylesheet is never
  loaded. See [Styling and theming](#styling-and-theming).

## The build

A module builds as an ES library with every engine package left **external** —
the host shell supplies exactly one runtime instance of each:

<<< @/../../examples/module-initiative-tracker/vite.config.ts

Output is `dist/index.js` (plus any chunks/assets your module splits), shipped
alongside your authored `module.json`.

::: warning The import map is a fixed set
`external` only tells Rollup not to bundle those specifiers — at runtime the
browser resolves them through the host's import map, which serves exactly these:

`svelte`, `svelte/internal/client`, `svelte/internal/disclose-version`,
`svelte/reactivity`, `@shadowcat/core`, `@shadowcat/ui-kit`,
`@shadowcat/formula`, `@shadowcat/types`.

Two failure modes to know:

1. **Unserved `svelte/*` subpath.** The `/^svelte\//` external keeps every svelte
   subpath out of your bundle, but if you import one the host does not serve —
   `svelte/store`, `svelte/transition`, `svelte/motion` — your module hard-fails
   at load with a runtime `SyntaxError`, not a build error. Adding a subpath to
   the runtime set is a host change; open an issue against Shadowcat.
2. **Package roots only.** The import map has exact-match entries for package
   roots. `@shadowcat/core/anything` is an unresolvable bare specifier and fails
   to load.
:::

## Registering a contribution

Your entry default-exports a `Module`: a manifest (mirroring `module.json`) plus
`register(ctx)`, called once at load. Registration means *contributing* a
component to a contract the host renders:

<<< @/../../examples/module-initiative-tracker/src/index.ts#manifest

<<< @/../../examples/module-initiative-tracker/src/index.ts#register

The `panel` metadata drives the panel-manager host: `icon` and `labelKey` render
the tab, `gmOnly: true` hides the panel from non-GM users (a UI filter — the
server still enforces real permissions on everything the panel does), and an
absent `defaultPlacement` means the panel starts launcher-only (closed).

`labelKey` is resolved against the host's i18n catalog. Register your module's own
messages via `ctx.i18n.addMessages(locale, messages)` before contributing the panel
(see `register` above, which registers `example-initiative-tracker.panelLabel`
before contributing it) — a later call for the same `(locale, key)` overwrites an
earlier one, including a built-in key, so prefix your keys with your module's id
by convention to avoid colliding with another module's or the host's own keys (this
is a convention, not an enforced uniqueness constraint). A key with no registered
message renders as its literal string.

## Styling and theming

The host UI is themed by CSS custom properties ("tokens") on the document root.
Users pick among built-in themes and can author their own, so **your module must
not hardcode colors** — a hex literal that looks right today goes dark-on-dark
under a different theme.

- **Consume the semantic tokens.** Reference the host's tier-2 tokens with plain
  `var(--*)` in your component styles: `--surface-base` / `--surface-raised` /
  `--surface-overlay` (backgrounds), `--text-primary` / `--text-muted`,
  `--accent` / `--accent-hover` / `--accent-active` / `--on-accent`, `--border`,
  `--danger` / `--on-danger`, `--success`, `--warning`, plus the spacing
  (`--space-*`), radius (`--radius-*`), and font-size (`--font-size-*`) scales.
  Your components then follow every built-in and user theme for free — including
  inside popped-out panel windows, which the host re-themes automatically.
- **Declare your stylesheet.** Svelte scoped `<style>` blocks compile to a CSS
  file your build emits separately from `index.js`; the host only loads it when
  the manifest's `style` field names it (see [The manifest](#the-manifest)).
  Pin the emitted name in your `vite.config.ts` (`build.lib.cssFileName`) so the
  manifest value is stable.
- **Opt out only deliberately.** A contribution may set `styling: "isolated"`
  to render inside the theme-isolation class, which re-declares every token at
  its engine-default value for that subtree — your module then re-implements
  surface/text/accent itself and ignores the user's theme entirely. The default
  (`"host"`) is what modules should almost always choose.

## Reading documents

Inside a contributed component, `getAppContext()` hands you the world session:
the document stores, your role, i18n, and every seam a module may touch.

<<< @/../../examples/module-initiative-tracker/src/InitiativePanel.svelte#read-actors

`ctx.documents` is the **optimistic** view (predictions included — what the user
should see *now*); `ctx.store` is the confirmed-only rollback base. Query by
document type, get by id.

::: warning The subscribe bridge is not optional
`ctx.documents` is a plain-callback store, not a Svelte rune. A `$derived` that
reads it without subscribing freezes at its first value and never updates —
worse, any edit built from that frozen read carries a stale concurrency
pre-image. Bridge it once with `createSubscriber` (as above) and call
`subscribe()` inside every `$derived.by` that reads the store.
:::

## Writing documents

Writes are **optimistic intents**: the client predicts the change locally and
transmits it; the server validates, applies, broadcasts — or rejects, rolling the
prediction back. The module-facing write helper is `setField`:

<<< @/../../examples/module-initiative-tracker/src/InitiativePanel.svelte#write-initiative

Three things this snippet does right — copy all three:

1. **JSON-pointer paths.** Fields are addressed as `/system/initiative`-style
   pointers. `/system` is the opaque, game-system-owned band of every document —
   modules may store their own data there.
2. **The OCC pre-image.** `setField`'s `old` argument must be the *raw current
   stored value* (`getPointer(actor, path)`) — the server uses it for optimistic
   concurrency control. Sending a fabricated `old` gets your write rejected on
   conflict.
3. **`ctx.canEdit` is advisory.** It mirrors the server's write check so you can
   hide dead controls; the server re-checks every write regardless.

## Install, enable, dev loop

**Install** (production): build, then copy `dist/index.js` + `module.json` (plus
your stylesheet if the manifest declares one) into the server's modules folder —

```
<data-dir>/modules/<folder-id>/
├── module.json
├── index.js
└── style.css   (when `"style"` is declared)
```

**Enable**: log in as GM → Settings → Installed modules → toggle your module on
for a world → reload. Enablement is per-world and takes effect on each client's
next load of that world (no hot enable/disable).

**Dev loop** (against a Shadowcat checkout):

1. Clone a Shadowcat checkout. Clone your module's repo into
   `src/modules/<your-id>/` inside it — the pnpm workspace glob resolves your
   `@shadowcat/*` deps and TS config with zero extra setup. Add
   `src/modules/<your-id>/` to the checkout's `.git/info/exclude` (git cannot
   pattern-match a directory that is its own nested repo).
2. Run a watch build whose output lands in `<data-dir>/modules/<your-id>/`
   (point it there via a `SHADOWCAT_MODULES_DIR` env var your `vite.config.ts`
   reads).
3. Run the Shadowcat dev server, enable your module in a dev world, reload.
   Your module always loads through the real modules-folder → server →
   import-map path — never a static import — matching production exactly.

## Testing

- **Unit tests** run in your module's own package with vitest against
  workspace-resolved `@shadowcat/*` packages (available once nested into a
  checkout, per the dev loop above). See `src/InitiativePanel.test.ts` in the
  example — plain logic exported from `index.ts` is the easiest surface to test.
- **End-to-end**: a Node script can drive the real Shadowcat `test_server`
  binary through the whole install → discover → enable → serve pipeline without
  a browser: build your module, stage its output as an installed module, spawn
  `test_server --modules-dir <staged-dir>`, log in, and assert the HTTP surface
  (`GET /api/modules`, the enable route, and the static entry serve).

## Reference

Generated API documentation for every symbol used above:

- [`Module`](/api/ts/interfaces/_shadowcat_core.Module.html) ·
  [`ModuleManifest`](/api/ts/interfaces/_shadowcat_core.ModuleManifest.html) ·
  [`PANEL_CONTRACT`](/api/ts/variables/_shadowcat_core.PANEL_CONTRACT.html) ·
  [`PanelMeta`](/api/ts/interfaces/_shadowcat_core.PanelMeta.html)
- [`getAppContext`](/api/ts/functions/_shadowcat_ui-kit.getAppContext.html) ·
  [`AppContext`](/api/ts/interfaces/_shadowcat_ui-kit.AppContext.html) ·
  [`setField`](/api/ts/functions/_shadowcat_ui-kit.setField.html)
- [`getPointer`](/api/ts/functions/_shadowcat_core.getPointer.html) ·
  [`ReadableDocuments`](/api/ts/interfaces/_shadowcat_core.ReadableDocuments.html) ·
  [`WireDocument`](/api/ts/types/_shadowcat_core.WireDocument.html)

Building a game system (custom sheets, rules, templates)? Continue with
[Creating a system](/guides/creating-a-system).
