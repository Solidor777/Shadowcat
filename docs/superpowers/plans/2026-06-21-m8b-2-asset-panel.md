# M8b-2 — Client Asset Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this repo's `CLAUDE.md` mandates
> **`mainline-plan-execution`** on this (Opus/Fable-class) model — use it INSTEAD of
> `superpowers:subagent-driven-development` / `superpowers:executing-plans`. Inline
> enumerative spec-compliance check per task + one final dispatched branch review.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** A minimal in-world asset panel — upload images, see a thumbnail grid of the
world's assets, select one (yields its UUID), replace or delete each — so file upload
is hand-testable in the running app, consuming the M8b-1 server endpoints and the
already-shipped `AssetResolver`.

**Architecture:** A new `Assets.svelte` panel contributed by the existing `core-ui`
module to the `shadowcat.surface:sidebar` contract (exactly like the `Settings`
panel). It reads `AppContext` for the world id, the `AssetResolver`, an
`onAssetChanged` subscription, and the i18n `t()`. New FormData-aware helpers in
`lib/api.ts` call the server. `WsClient` gains an `asset_changed` case that fans out to
an `onAssetChanged` handler; `WorldSession` owns the `AssetResolver`, applies each
`AssetChanged` to it, and notifies panel subscribers so thumbnails re-resolve. A
Playwright smoke drives upload→thumbnail→replace→delete against the built binary.

**Tech Stack:** Svelte 5 (runes, snippets, `onclick`/`onchange` attributes),
TypeScript, Vite, `@shadowcat/core` + `@shadowcat/ui` (pnpm workspace),
`@shadowcat/types` (ts-rs `Asset`). Tests: Vitest + `@testing-library/svelte` (jsdom)
for units, Playwright against the binary for e2e.

## Global Constraints

- **Mobile/touch-ready:** the panel reflows on a phone and its tap targets are
  touch-sized (project cross-platform directive). No hover-only affordances.
- **Styling via the 3-tier SCSS tokens** (M7d): use `var(--space-*)`,
  `var(--text-*)`, `var(--surface-*)` etc. — never hardcoded colors/spacing.
- **All user-facing copy via i18n `t()`** — no literal strings in markup (M7d
  framework-neutral i18n).
- **Svelte 5 only:** runes (`$state`/`$derived`/`$effect`/`$props`), event attributes
  (`onclick`), snippets — never `export let`, `$:`, or `on:click`.
- **Client logging through the project logger**, never raw `console.log`.
- **embed-dist ordering:** the server binary embeds `../../dist/`; any binary/e2e
  build runs `vite build` before `cargo build` (the `e2e:build` script already does).

---

## File Structure

- **Modify** `src/client/ui/src/lib/api.ts` — `uploadAsset`/`listAssets`/`replaceAsset`/`deleteAsset` (FormData).
- **Modify** `src/client/core/src/ws-client.ts` — `onAssetChanged` handler + `asset_changed` case.
- **Modify** `src/client/ui/src/lib/worldSession.svelte.ts` — own `AssetResolver`, wire the handler, expose `onAssetChanged`.
- **Modify** `src/client/ui/src/lib/appContext.ts` — add `assets` + `onAssetChanged` to `AppContext`.
- **Modify** `src/client/ui/src/lib/Table.svelte` — pass the two new context fields.
- **Modify** `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte` — supply the new fields (type-correctness).
- **Create** `src/client/ui/src/modules/core-ui/panels/Assets.svelte` — the panel.
- **Modify** `src/client/ui/src/modules/core-ui/index.ts` — contribute the panel.
- **Modify** `src/client/ui/src/locales/en.ts` — `assets.*` strings.
- **Create** `src/client/ui/src/modules/core-ui/panels/Assets.test.ts` + `__fixtures__/AssetsHarness.svelte` — unit tests.
- **Modify** `src/client/core/src/ws-client.test.ts` — `asset_changed` dispatch test.
- **Create** `src/client/ui/e2e/assets.spec.ts` — Playwright smoke.

---

## Task 1: API client — asset endpoints (FormData)

**Files:**
- Modify: `src/client/ui/src/lib/api.ts`
- Test: `src/client/ui/src/lib/api.test.ts` (create)

**Interfaces:**
- Produces:
  - `uploadAsset(world: string, file: File): Promise<Asset>`
  - `listAssets(world: string): Promise<Asset[]>`
  - `replaceAsset(uuid: string, file: File): Promise<Asset>`
  - `deleteAsset(uuid: string): Promise<void>`
  - (`Asset` imported from `@shadowcat/types`.)

- [ ] **Step 1: Write the failing test**

Create `src/client/ui/src/lib/api.test.ts`:

```typescript
import { describe, it, expect, vi, afterEach } from "vitest";
import { uploadAsset, listAssets, deleteAsset } from "./api";

afterEach(() => vi.restoreAllMocks());

describe("asset api", () => {
  it("uploadAsset POSTs multipart FormData and returns the asset", async () => {
    const asset = { id: "a1", world_id: "w1", version: 1 };
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(JSON.stringify(asset), { status: 200 }));
    const file = new File([new Uint8Array([1, 2, 3])], "x.png", { type: "image/png" });

    const out = await uploadAsset("w1", file);

    expect(out).toEqual(asset);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/worlds/w1/assets");
    expect((init as RequestInit).method).toBe("POST");
    expect((init as RequestInit).body).toBeInstanceOf(FormData);
  });

  it("listAssets GETs the per-world list", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify([{ id: "a1" }]), { status: 200 }),
    );
    const out = await listAssets("w1");
    expect(out).toHaveLength(1);
  });

  it("deleteAsset throws on a non-ok status", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", { status: 403 }));
    await expect(deleteAsset("a1")).rejects.toThrow();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test api`
Expected: FAIL — `uploadAsset` not exported.

- [ ] **Step 3: Implement the helpers**

In `src/client/ui/src/lib/api.ts`, add the import at the top (beside the other
`@shadowcat/types` imports) and the four helpers at the end:

```typescript
import type { Asset } from "@shadowcat/types";

/** Upload an image to a world; returns the created asset record. */
export async function uploadAsset(world: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/worlds/${world}/assets`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`upload failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** List a world's assets (the grid source). */
export async function listAssets(world: string): Promise<Asset[]> {
  const res = await fetch(`/api/worlds/${world}/assets`);
  if (!res.ok) throw new Error(`list failed: ${res.status}`);
  return (await res.json()) as Asset[];
}

/** Replace an asset's bytes behind its stable UUID; returns the updated record. */
export async function replaceAsset(uuid: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/assets/${uuid}/replace`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`replace failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** Delete an asset (file + record). */
export async function deleteAsset(uuid: string): Promise<void> {
  const res = await fetch(`/api/assets/${uuid}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`delete failed: ${res.status}`);
}
```

(If `api.ts` already imports types from `@shadowcat/types`, merge `Asset` into the
existing import rather than adding a second line. `fetch` uses same-origin cookies by
default, matching the existing `getJson`/`postJson` calls.)

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/ui test api`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/api.ts src/client/ui/src/lib/api.test.ts
git commit -m "feat(m8b): client api helpers for asset upload/list/replace/delete"
```

---

## Task 2: WsClient — `asset_changed` dispatch

**Files:**
- Modify: `src/client/core/src/ws-client.ts`
- Test: `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Produces: `WsClientHandlers.onAssetChanged?(msg: { uuid: string; op: "replaced" | "deleted" }): void`
- Consumes: the `asset_changed` variant of `ServerMsg` (already in `wire.ts`).

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/ws-client.test.ts` (match the file's existing harness for
constructing a `WsClient` with a mock transport and pushing a frame — reuse the same
helper the `search_update`/`scene_derived` tests use to deliver an inbound frame):

```typescript
it("dispatches asset_changed frames to onAssetChanged", async () => {
  const seen: Array<{ uuid: string; op: string }> = [];
  // `harness` here stands for the file's existing setup that wires a mock
  // transport and returns a way to push a raw inbound frame; mirror the
  // search_update test exactly.
  const h = makeClient({ onAssetChanged: (m) => seen.push(m) });
  await h.start();
  h.deliver(JSON.stringify({ type: "asset_changed", uuid: "a1", op: "replaced" }));
  expect(seen).toEqual([{ uuid: "a1", op: "replaced" }]);
});
```

(Adapt `makeClient`/`deliver` to the real helper names in this test file — read the
existing `scene_derived`/`search_update` test in `ws-client.test.ts` and copy its
exact construction + frame-delivery mechanism. The assertion is the load-bearing part.)

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/core test ws-client`
Expected: FAIL — `onAssetChanged` never invoked (no case).

- [ ] **Step 3: Add the handler field and the case**

In `src/client/core/src/ws-client.ts`, add to `interface WsClientHandlers` (beside
`onWelcome?`/`onError?`):

```typescript
  /** An out-of-band asset mutation notice (replace/delete); carries no seq. */
  onAssetChanged?(msg: { uuid: string; op: "replaced" | "deleted" }): void;
```

In `handleFrame()`'s `switch (msg.type)` (the block that has `case "search_update"`,
`case "scene_derived"`, etc.), add:

```typescript
      case "asset_changed":
        this.safeEmit(() => this.opts.handlers.onAssetChanged?.({ uuid: msg.uuid, op: msg.op }));
        break;
```

(Use the same dispatch wrapper the neighboring cases use — if they call
`this.safeEmit(...)`, match it; if they call the handler directly, match that. Read the
adjacent cases and mirror them exactly.)

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/core test ws-client`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts
git commit -m "feat(m8b): WsClient dispatches asset_changed to onAssetChanged"
```

---

## Task 3: WorldSession — own the resolver, wire the broadcast

**Files:**
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts`
- Test: `src/client/ui/src/lib/worldSession.test.ts`

**Interfaces:**
- Consumes: `AssetResolver` (from `@shadowcat/core`), `WsClientHandlers.onAssetChanged` (Task 2).
- Produces on `WorldSession`:
  - `readonly assets: AssetResolver`
  - `onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void`

- [ ] **Step 1: Write the failing test**

Add to `src/client/ui/src/lib/worldSession.test.ts` (reuse the file's existing
mock-`connect` harness that lets a test deliver a frame; mirror how its Welcome/command
tests inject inbound frames):

```typescript
it("applies asset_changed to the resolver and notifies subscribers", async () => {
  const session = makeSession(); // the file's existing constructor helper
  const got: Array<{ uuid: string; op: string }> = [];
  session.onAssetChanged((m) => got.push(m));
  await session.enter("w1");

  const before = session.assets.url("a1"); // "/api/assets/a1"
  deliverFrame({ type: "asset_changed", uuid: "a1", op: "replaced" }); // file's helper
  // Resolver cache-busts on replace, and subscribers are notified.
  expect(session.assets.url("a1")).not.toBe(before);
  expect(got).toEqual([{ uuid: "a1", op: "replaced" }]);
});
```

(Adapt `makeSession`/`deliverFrame` to the real helpers in `worldSession.test.ts` —
read the existing tests and copy their session construction + frame-delivery exactly.)

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test worldSession`
Expected: FAIL — `onAssetChanged`/`assets` missing.

- [ ] **Step 3: Implement**

In `src/client/ui/src/lib/worldSession.svelte.ts`:

Add `AssetResolver` to the `@shadowcat/core` import list. Add the field + listener set
to the class (beside `readonly contributions`):

```typescript
  readonly assets = new AssetResolver();
  #assetListeners = new Set<(msg: { uuid: string; op: "replaced" | "deleted" }) => void>();

  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void {
    this.#assetListeners.add(cb);
    return () => this.#assetListeners.delete(cb);
  }
```

In `enter()`, add the handler to the `WsClient` `handlers` object (beside
`onError`):

```typescript
        onAssetChanged: (msg) => {
          // Bump the resolver first so a notified panel re-resolves the new URL.
          this.assets.onAssetChanged(msg);
          for (const cb of this.#assetListeners) cb(msg);
        },
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/ui test worldSession`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/worldSession.svelte.ts src/client/ui/src/lib/worldSession.test.ts
git commit -m "feat(m8b): WorldSession owns AssetResolver + fans out AssetChanged"
```

---

## Task 4: AppContext — expose `assets` + `onAssetChanged`

**Files:**
- Modify: `src/client/ui/src/lib/appContext.ts`
- Modify: `src/client/ui/src/lib/Table.svelte`
- Modify: `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte`

**Interfaces:**
- Produces on `AppContext`:
  - `assets: AssetResolver`
  - `onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void`

- [ ] **Step 1: Extend the `AppContext` type**

In `src/client/ui/src/lib/appContext.ts`, add `AssetResolver` to the `@shadowcat/core`
type import and two fields to `interface AppContext` (after `t`):

```typescript
  /** Resolves asset UUIDs to serve URLs, cache-busting on replace. */
  assets: AssetResolver;
  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void;
```

(Change `import type { ContributionRegistry, DocumentStore } from "@shadowcat/core";`
to also import `AssetResolver`.)

- [ ] **Step 2: Pass them from the provider (`Table.svelte`)**

In `src/client/ui/src/lib/Table.svelte`, extend the `setAppContext({...})` call:

```typescript
  setAppContext({
    contributions: session.contributions,
    store: session.store,
    world: session.world!,
    role: session.role!,
    t,
    leaveWorld,
    assets: session.assets,
    onAssetChanged: (cb) => session.onAssetChanged(cb),
  });
```

(Keep the existing `svelte-ignore state_referenced_locally` comment; `session.assets`
is a fixed instance like `session.store`.)

- [ ] **Step 3: Satisfy the test fixture**

In `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte`, add the two fields so the
`AppContext` shape type-checks (the fixture exercises `<Surface>`, which ignores them):

```typescript
  import { ContributionRegistry, DocumentStore, AssetResolver } from "@shadowcat/core";
  // ...
  setAppContext({
    contributions: registry,
    store: new DocumentStore(),
    world: "test",
    role: "gm",
    t,
    leaveWorld: () => {},
    assets: new AssetResolver(),
    onAssetChanged: () => () => {},
  });
```

- [ ] **Step 4: Verify type-check + existing tests pass**

Run: `pnpm --filter @shadowcat/ui typecheck`
then: `pnpm --filter @shadowcat/ui test Surface`
Expected: both green (the AppContext shape now includes the new fields; `<Surface>`
tests unaffected).

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/appContext.ts src/client/ui/src/lib/Table.svelte src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte
git commit -m "feat(m8b): AppContext exposes AssetResolver + onAssetChanged"
```

---

## Task 5: The Assets panel + contribution + i18n

**Files:**
- Create: `src/client/ui/src/modules/core-ui/panels/Assets.svelte`
- Modify: `src/client/ui/src/modules/core-ui/index.ts`
- Modify: `src/client/ui/src/locales/en.ts`
- Create: `src/client/ui/src/modules/core-ui/panels/__fixtures__/AssetsHarness.svelte`
- Create: `src/client/ui/src/modules/core-ui/panels/Assets.test.ts`

**Interfaces:**
- Consumes: `getAppContext()` (`world`, `assets`, `onAssetChanged`, `t`), the Task 1 api
  helpers, `Asset` from `@shadowcat/types`.
- Produces: the `Assets` Svelte component, contributed as `core-ui:assets` to
  `shadowcat.surface:sidebar`.

- [ ] **Step 1: Add the i18n strings**

In `src/client/ui/src/locales/en.ts`, add an `assets` group (match the file's existing
nested-object shape and the `settings`/`world` groups):

```typescript
  assets: {
    title: "Assets",
    upload: "Upload image",
    replace: "Replace",
    delete: "Delete",
    empty: "No assets yet.",
    selected: "Selected: {id}",
    error: "Asset operation failed: {message}",
  },
```

- [ ] **Step 2: Write the failing panel test**

Create the harness `src/client/ui/src/modules/core-ui/panels/__fixtures__/AssetsHarness.svelte`:

```svelte
<script lang="ts">
  import { AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "../../../../lib/appContext";
  import { t } from "../../../../lib/i18n.svelte";
  import Assets from "../Assets.svelte";

  let { onAssetChanged = () => () => {} }: {
    onAssetChanged?: (cb: (m: { uuid: string; op: "replaced" | "deleted" }) => void) => () => void;
  } = $props();
  // svelte-ignore state_referenced_locally
  setAppContext({
    contributions: undefined as never,
    store: undefined as never,
    world: "w1",
    role: "gm",
    t,
    leaveWorld: () => {},
    assets: new AssetResolver(),
    onAssetChanged,
  });
</script>

<Assets />
```

Create `src/client/ui/src/modules/core-ui/panels/Assets.test.ts`:

```typescript
import { render, screen, waitFor, fireEvent } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import Harness from "./__fixtures__/AssetsHarness.svelte";
import * as api from "../../../lib/api";

beforeEach(() => vi.restoreAllMocks());

test("renders a thumbnail grid from listAssets", async () => {
  vi.spyOn(api, "listAssets").mockResolvedValue([
    { id: "a1", world_id: "w1", storage_key: "", original_name: "map.png",
      content_type: "image/png", byte_size: 1, created_by: "u", created_at: 0, version: 1 },
  ] as never);
  render(Harness);
  const tile = await screen.findByTestId("asset-tile");
  expect(tile).toBeTruthy();
  expect(screen.getByText("map.png")).toBeTruthy();
});

test("uploading a file calls uploadAsset then reloads", async () => {
  vi.spyOn(api, "listAssets").mockResolvedValue([] as never);
  const upload = vi.spyOn(api, "uploadAsset").mockResolvedValue({ id: "a1" } as never);
  render(Harness);
  const input = await screen.findByTestId("asset-upload");
  const file = new File([new Uint8Array([1])], "x.png", { type: "image/png" });
  await fireEvent.change(input, { target: { files: [file] } });
  await waitFor(() => expect(upload).toHaveBeenCalledWith("w1", file));
});

test("an asset_changed notice triggers a reload", async () => {
  const list = vi.spyOn(api, "listAssets").mockResolvedValue([] as never);
  let fire: (m: { uuid: string; op: "replaced" | "deleted" }) => void = () => {};
  render(Harness, { props: { onAssetChanged: (cb: typeof fire) => { fire = cb; return () => {}; } } });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  fire({ uuid: "a1", op: "deleted" });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
});
```

- [ ] **Step 3: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test Assets`
Expected: FAIL — `../Assets.svelte` does not exist.

- [ ] **Step 4: Implement the panel**

Create `src/client/ui/src/modules/core-ui/panels/Assets.svelte`:

```svelte
<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "../../../lib/appContext";
  import { listAssets, uploadAsset, replaceAsset, deleteAsset } from "../../../lib/api";

  const { world, assets: resolver, onAssetChanged, t } = getAppContext();

  let items = $state<Asset[]>([]);
  let selectedId = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function reload(): Promise<void> {
    try {
      items = await listAssets(world);
      error = null;
    } catch (e) {
      error = t("assets.error", { message: String(e) });
    }
  }

  // Load on mount; reload whenever another client (or our own replace/delete)
  // broadcasts an AssetChanged. The resolver was already cache-busted by
  // WorldSession before this fires, so re-rendered <img> tags pull fresh bytes.
  $effect(() => {
    void reload();
    return onAssetChanged(() => void reload());
  });

  async function onUpload(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    try {
      await uploadAsset(world, file);
      await reload();
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }

  async function onReplace(uuid: string, e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    try {
      await replaceAsset(uuid, file);
      // The asset_changed{replaced} broadcast drives the reload + cache-bust.
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }

  async function onDelete(uuid: string): Promise<void> {
    try {
      await deleteAsset(uuid);
      if (selectedId === uuid) selectedId = null;
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }
</script>

<section class="assets">
  <h2>{t("assets.title")}</h2>

  <label class="upload">
    <span>{t("assets.upload")}</span>
    <input type="file" accept="image/*" onchange={onUpload} data-testid="asset-upload" />
  </label>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if items.length === 0}
    <p class="empty">{t("assets.empty")}</p>
  {:else}
    <ul class="grid">
      {#each items as a (a.id)}
        <li class="tile" class:selected={selectedId === a.id} data-testid="asset-tile">
          <button class="thumb" type="button" onclick={() => (selectedId = a.id)}>
            <img src={resolver.url(a.id)} alt={a.original_name} />
          </button>
          <span class="name">{a.original_name}</span>
          <div class="row">
            <label class="replace">
              <span>{t("assets.replace")}</span>
              <input type="file" accept="image/*" onchange={(e) => onReplace(a.id, e)} />
            </label>
            <button type="button" onclick={() => onDelete(a.id)}>{t("assets.delete")}</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  {#if selectedId}
    <p class="selected" data-testid="selected-id">{t("assets.selected", { id: selectedId })}</p>
  {/if}
</section>

<style lang="scss">
  .assets {
    padding: var(--space-4);
    display: grid;
    gap: var(--space-3);
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7rem, 1fr));
    gap: var(--space-3);
  }
  .tile {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--surface-border);
    border-radius: var(--radius-2);
  }
  .tile.selected {
    border-color: var(--accent-base);
  }
  .thumb {
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
  }
  .thumb img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: var(--radius-1);
    display: block;
  }
  .name {
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  .error {
    color: var(--danger-base);
  }
</style>
```

(The SCSS token names — `--surface-border`, `--accent-base`, `--radius-1/2`,
`--text-sm`, `--danger-base` — must match the M7d token set; read
`src/client/ui/src/styles` (or the tokens file) and substitute the real names if any
differ. Do not invent new tokens.)

- [ ] **Step 5: Contribute the panel from core-ui**

In `src/client/ui/src/modules/core-ui/index.ts`, import the panel and add a
contribution inside `register(ctx)` (beside the `core-ui:settings` one):

```typescript
import Assets from "./panels/Assets.svelte";
// ... inside register(ctx):
    ctx.contributions.contribute({
      id: "core-ui:assets",
      contract: "shadowcat.surface:sidebar",
      order: 1,
      component: Assets,
    });
```

- [ ] **Step 6: Run to verify it passes + typecheck**

Run: `pnpm --filter @shadowcat/ui test Assets`
then: `pnpm --filter @shadowcat/ui typecheck`
Expected: PASS / green.

- [ ] **Step 7: Commit**

```bash
git add src/client/ui/src/modules/core-ui/ src/client/ui/src/locales/en.ts
git commit -m "feat(m8b): assets sidebar panel — upload/grid/select/replace/delete"
```

---

## Task 6: Playwright smoke — upload → thumbnail → replace → delete

**Files:**
- Create: `src/client/ui/e2e/assets.spec.ts`

**Interfaces:**
- Consumes: the running binary (served SPA + asset endpoints); the panel from Task 5.

- [ ] **Step 1: Write the e2e smoke**

Create `src/client/ui/e2e/assets.spec.ts` (model the login+enter flow on the existing
`e2e/entry-flow.spec.ts`):

```typescript
import { test, expect } from "@playwright/test";

// A 1×1 PNG as bytes, written to a temp file Playwright can upload.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

test("upload an image, see the thumbnail, replace it, then delete it", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Asset World");
  await page.getByRole("button", { name: "Create world" }).click();

  // In-world: the Assets panel is in the sidebar.
  await expect(page.getByRole("heading", { name: "Assets" })).toBeVisible();

  // Upload.
  await page
    .getByTestId("asset-upload")
    .setInputFiles({ name: "map.png", mimeType: "image/png", buffer: PNG_1X1 });
  const tile = page.getByTestId("asset-tile");
  await expect(tile).toHaveCount(1);

  // Replace (the tile persists; same UUID, new bytes).
  await tile.locator('input[type="file"]').setInputFiles({
    name: "map2.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Delete.
  await tile.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByTestId("asset-tile")).toHaveCount(0);
});
```

- [ ] **Step 2: Build the binary (client-first) and run the smoke**

Run: `pnpm --filter @shadowcat/ui e2e`
(`e2e:build` runs `vite build` then `cargo build -p shadowcat`; Playwright launches the
binary per `playwright.config.ts`.)
Expected: PASS — the asset loop works end-to-end against the binary.

- [ ] **Step 3: Commit**

```bash
git add src/client/ui/e2e/assets.spec.ts
git commit -m "test(m8b): Playwright smoke over the asset upload->replace->delete loop"
```

---

## Final verification gates (before declaring M8b-2 complete)

- [ ] `pnpm --filter @shadowcat/core test` and `pnpm --filter @shadowcat/ui test` — green.
- [ ] `pnpm --filter @shadowcat/core typecheck` and `pnpm --filter @shadowcat/ui typecheck` — green.
- [ ] `pnpm lint` — green.
- [ ] `pnpm --filter @shadowcat/ui e2e` — the asset smoke passes against the binary.
- [ ] `cargo test -p shadowcat` — server still green (no server changes here, but the
      e2e build recompiles the binary; confirm nothing drifted).
- [ ] `git diff --exit-code src/types/generated` — no ts-rs drift (no server type change expected).
- [ ] `graphify update .` after code changes.

---

## Self-Review (completed during authoring)

- **Spec coverage (§7 minimal asset panel):** upload → Task 1/5; thumbnail grid →
  Task 5; select-to-use (yields UUID) → Task 5 (`selectedId` + `assets.selected`);
  replace/delete → Task 1/5; re-resolve on `AssetChanged` → Task 2/3/5; mounted on a
  core-ui surface → Task 5; Playwright smoke over the loop → Task 6. §8 decomposition:
  this is M8b-2 (client); M8b-1 (server) already shipped.
- **Placeholder scan:** the only adaptation notes are the three test-harness helpers
  in Tasks 2 & 3 (`makeClient`/`deliver`, `makeSession`/`deliverFrame`) — these point
  the implementer at the *existing* mock-transport helpers in the real test files to
  copy, not at unspecified behavior; the assertions and production code are fully
  given. SCSS token names flagged to verify against the real M7d token file.
- **Type consistency:** `Asset` (from `@shadowcat/types`), the `{ uuid, op:
  "replaced" | "deleted" }` message shape, `onAssetChanged(cb): () => void`,
  `assets: AssetResolver`, and the api helper signatures are used identically across
  Tasks 1–6. The contribution uses `shadowcat.surface:sidebar` (the real contract from
  core-ui) with `order: 1` (Settings is `order: 0`).
