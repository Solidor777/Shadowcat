# Authoring an External Shadowcat Module

This guide covers the build toolchain and dev workflow for a module built
OUTSIDE the Shadowcat repository (its own git repo, its own release cycle).
Nightfox (`C:\Dev\Nightfox` in this checkout's development environment) is the
reference implementation — copy its file layout for a new module.

## Manifest (`module.json`)

A module ships one `module.json` at its install-folder root. Its shape is the
client `ModuleManifest` (`src/client/core/src/manifest.ts`) — the server serves
it byte-for-byte at `GET /api/modules`, so the client's own Zod schema sees
every field you declare (`dependencies`, `hooks`, `provides`, `requires`, ...).
Every community module MUST additionally set `engines`:

```json
{
  "id": "your-module-id",
  "version": "0.1.0",
  "engines": { "shadowcat": "^0.1.0" },
  "dependencies": {},
  "capabilities": [],
  "requirements": [],
  "provides": [],
  "requires": []
}
```

- `engines.shadowcat` is a semver range (exact / `^` / `~` / `*`), checked
  against the running server's version at both enable time (a GM toggles it on
  in a world) and load time (a client actually imports it). Missing this field
  = the module can never be enabled.
- `entry` (optional, default `"index.js"`) overrides the built entry file name,
  relative to the module's install folder. This field is read by the server's
  module scanner (`src/server/src/modules.rs`) to compute the served entry URL
  (`/modules/<folder-id>/<entry>`) — it is not part of the client
  `ModuleManifest` Zod shape, so declare it in `module.json` only when your
  build emits something other than `index.js`.
- `requirements` (declarative path-prefix → capability rules) are unioned into
  the world's broadcast `capability_requirements` for every world where the
  module is enabled — no separate publish step. They are advisory to the client
  only; the server never enforces third-party-declared requirements (ARCHITECTURE
  invariant 6 — authority over the world's real capability rules stays with the
  GM's `world_cap_requirements`).

The module's **install-folder name is its identity** for the enabled-module set,
not the `id` declared here. The server keys the per-world enabled set on the
folder name (which it controls); your declared `id` is cross-checked client-side
but never trusted as the persistence key. Keep the folder name and `id` equal to
avoid confusion.

## Build config (Vite)

A module builds as an ES library with every engine package left external —
the host (Shadowcat's shell) supplies exactly one instance of each at runtime
(Global Constraint 1: exactly one `svelte` / `@shadowcat/*` instance):

```ts
// vite.config.ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: {
    lib: { entry: "src/index.ts", formats: ["es"], fileName: () => "index.js" },
    rollupOptions: {
      external: [
        "svelte",
        /^svelte\//,
        "@shadowcat/core",
        "@shadowcat/ui-kit",
        "@shadowcat/formula",
        "@shadowcat/types",
      ],
    },
  },
});
```

Output = `dist/index.js` (+ any chunks/assets your module itself splits) plus
your authored `module.json`, copied through unchanged. See Nightfox's
`scripts/copy-manifest.mjs` for the copy step.

### Which externals actually resolve at runtime

`external` tells Rollup not to bundle these; the browser resolves them through
the host's import map (`src/client/shell/index.html`, generated from
`RUNTIME_ENTRIES` in `src/client/shell/vite.config.ts`). The host serves a
**fixed** set of runtime chunks, so only these bare specifiers resolve:

- `svelte`
- `svelte/internal/client`
- `svelte/internal/disclose-version`
- `svelte/reactivity`
- `@shadowcat/core`
- `@shadowcat/ui-kit`
- `@shadowcat/formula`
- `@shadowcat/types`

The `/^svelte\//` external is a build-time convenience: it keeps every
`svelte/*` subpath out of your bundle. But if you import a subpath the host
does NOT serve — `svelte/store`, `svelte/transition`, `svelte/motion`, etc. —
the import map has no entry and your module hard-fails with a runtime
`SyntaxError` on load, not a build error. Adding a new `svelte/*` subpath to the
runtime set is a **host change** (extend `RUNTIME_ENTRIES` + the import map in
`src/client/shell`), not something a module can do on its own. Stick to the set
above, or open an issue against Shadowcat to have the subpath added.

## Dev flow (parity: never statically bundled, even in dev)

1. Clone a Shadowcat checkout. Clone your module's repo into
   `src/modules/<your-id>/` inside it — the pnpm workspace glob
   (`src/modules/*`) resolves `@shadowcat/*` and TS config with zero extra
   setup. Add `src/modules/<your-id>/` to the checkout's `.git/info/exclude`
   (git cannot pattern-match "a directory that is its own nested repo").
2. `pnpm --filter <your-id> dev` — a watch build whose output lands in
   `<data-dir>/modules/<your-id>/` (point it there via the
   `SHADOWCAT_MODULES_DIR` env var your `vite.config.ts` reads, matching
   Nightfox's template).
3. Run the Shadowcat dev server; log in as GM; open Settings → Installed
   modules; enable your module in a dev world; reload. Your module ALWAYS
   loads through the real modules-folder → server → import-map path, never a
   static import — matching production exactly.

## Testing

- **Unit tests** run in your module's own repo with vitest, against
  workspace-resolved `@shadowcat/*` packages (only available once nested into
  a checkout, per step 1 above — a module repo cloned standalone cannot
  `pnpm install` its `@shadowcat/*` deps).
- **e2e access**: a Node script in your repo can drive the real Shadowcat
  `test_server` binary end to end (install → discover → enable → serve),
  without a browser. See Nightfox's `e2e/run-e2e.mjs` for a complete,
  copy-pasteable template: it builds your module, stages its output as an
  installed module, spawns `test_server --modules-dir <staged-dir>`, logs in,
  and asserts the full HTTP surface (`GET /api/modules`, `PUT
  .../enabled-modules`, and the static entry serve).

## Known limits (M13-1)

- No upload/install UI — install is manual folder extraction into
  `<data-dir>/modules/<folder-id>/`.
- No sandboxing — an installed module is admin-trusted client code, the same
  trust tier as the server binary itself.
- No hot enable/disable — a change takes effect on the affected client's next
  load of that world (page reload / re-enter), not instantly for an
  already-open session.
