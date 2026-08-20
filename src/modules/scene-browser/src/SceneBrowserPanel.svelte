<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildSceneDoc, listAssets, type WireDocument, type WorldSettingsEngine, type SceneEngine } from "@shadowcat/core";
  import type { Asset } from "@shadowcat/types";

  const ctx = getAppContext();
  const t = ctx.t;

  let assetList = $state<Asset[]>([]);

  /**
   * Refetches the world's image assets (filtered to `image/*` content types) into `assetList`,
   * feeding the background-picker snippet. Called once at mount and on every `AssetChanged`
   * broadcast, via the `$effect` below.
   * @returns Nothing; assigns the component's own `assetList` `$state`.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the `$effect` below
   * refreshAssets();
   * ```
   */
  function refreshAssets(): void {
    void listAssets(ctx.world).then((a) => {
      assetList = a.filter((x) => x.content_type.startsWith("image/"));
      ctx.assets.reconcile(a);
    });
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  /** The scene id whose background picker is currently open, or `null` when none is. */
  let pickerOpenFor = $state<string | null>(null);

  // Reactive bridge (mandatory): register a dependency on the doc store so the list re-renders on
  // create/activate and the viewed/active badges track edits.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const scenes = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("scene");
  });
  const ws = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const activeSceneId = $derived.by((): string | null => {
    return (ws?.engine as WorldSettingsEngine | undefined)?.activeScene ?? null;
  });
  // The GM's own rendered scene (roam or followed active). Reading the getter tracks the doc store
  // (via subscribe above) + the session's gmViewedScene state.
  const viewedSceneId = $derived.by((): string | null => {
    subscribe();
    return ctx.viewedSceneId;
  });
  const roaming = $derived(viewedSceneId !== null && viewedSceneId !== activeSceneId);

  /**
   * The scene's background asset id, if any — feeds the list row's thumbnail `<img>` src.
   * @param scene The scene document to read `engine.background` from.
   * @returns The background asset id, or `null` if the scene has none.
   * @example
   * ```
   * declare const sceneDoc: WireDocument;
   * // private helper; not part of the public API
   * bgOf(sceneDoc);
   * ```
   */
  function bgOf(scene: WireDocument): string | null {
    return (scene.engine as SceneEngine | undefined)?.background ?? null;
  }

  /**
   * **Activate** — sets the scene every player (and any non-roaming GM) renders, by updating the
   * world-settings document's `engine.activeScene`. Scope: EVERY connected client, not just this
   * one — distinct from `view` below, which is local to this client only. OCC pre-image is the
   * REAL current activeScene (or `null` when genuinely absent) — never a defaulted value. Silent
   * no-op if world-settings is absent (game-settings seeds it on the same GM Welcome, so this is
   * a narrow startup race, not a steady-state condition).
   * @param sceneId The scene document id to activate.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * declare const sceneId: string;
   * // private helper; not part of the public API — invoked from a scene row's "activate" button
   * activate(sceneId);
   * ```
   */
  function activate(sceneId: string): void {
    if (!ws) return;
    const old = (ws.engine as WorldSettingsEngine | undefined)?.activeScene ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: ws.id, changes: [{ path: "/engine/activeScene", old, new: sceneId }] }]);
  }

  /**
   * **Local view** (GM roam) — changes only THIS client's viewed scene, via
   * `ctx.setGmViewedScene`; every other player keeps rendering the world's `activeScene`,
   * unaffected. Distinct from `activate` above, which is world-wide.
   * @param sceneId The scene document id to view locally.
   * @returns Nothing; delegates to `ctx.setGmViewedScene`.
   * @example
   * ```
   * declare const sceneId: string;
   * // private helper; not part of the public API — invoked from a scene row's "view" button
   * view(sceneId);
   * ```
   */
  function view(sceneId: string): void {
    ctx.setGmViewedScene(sceneId);
  }
  /**
   * Stops local roaming and resumes following the world's `activeScene`, by clearing the GM's
   * viewed-scene override (`ctx.setGmViewedScene(null)`). Scope: this client only, same as
   * `view` above — the inverse of entering roam.
   * @returns Nothing; delegates to `ctx.setGmViewedScene`.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the roaming banner's button
   * followActive();
   * ```
   */
  function followActive(): void {
    ctx.setGmViewedScene(null);
  }

  /**
   * **Configure** — deep-links the game-settings panel to this scene's per-scene section.
   * Changes no document and no other client's view, unlike `activate`/`view` above: it only
   * selects the scene in `ctx.sceneSelection` (read by the game-settings panel to choose its
   * section) and opens that panel.
   * @param sceneId The scene document id to deep-link to.
   * @returns Nothing; side effects only (selection + panel open).
   * @example
   * ```
   * declare const sceneId: string;
   * // private helper; not part of the public API — invoked from a scene row's "configure" button
   * configure(sceneId);
   * ```
   */
  function configure(sceneId: string): void {
    ctx.sceneSelection.select(sceneId);
    ctx.panels.open("game-settings:panel");
  }

  /**
   * Creates a new, empty scene document in the current world. Does not activate or view it — a
   * GM must separately `activate`/`view` the new scene once it appears in the list.
   * @returns Nothing; dispatches a create intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the "create" button
   * create();
   * ```
   */
  function create(): void {
    ctx.dispatchIntent([{ op: "create", doc: buildSceneDoc(ctx.world) }]);
  }

  /**
   * Sets (or clears, when `assetId` is `null`) a scene's `engine.background`, OCC-dispatched with
   * the RAW current stored value (via `bgOf`) as `old` — mirrors `activate()`'s convention.
   * @param scene The scene document to update.
   * @param assetId The new background asset id, or `null` to clear it.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * declare const scene: WireDocument;
   * // private helper; not part of the public API — invoked from the background picker
   * setBackground(scene, "asset-1");
   * ```
   */
  function setBackground(scene: WireDocument, assetId: string | null): void {
    ctx.dispatchIntent([{ op: "update", doc_id: scene.id, changes: [{ path: "/engine/background", old: bgOf(scene), new: assetId }] }]);
    pickerOpenFor = null;
  }
</script>

<section class="scene-browser" aria-label={t("sceneBrowser.title")}>
  <h3>{t("sceneBrowser.title")}</h3>
  {#if roaming}
    <p class="hint">
      {t("sceneBrowser.roaming")}
      <button type="button" onclick={followActive}>{t("sceneBrowser.followActive")}</button>
    </p>
  {/if}
  <ul class="list">
    {#each scenes as scene, i (scene.id)}
      <li class:active={scene.id === activeSceneId} class:viewed={scene.id === viewedSceneId}>
        <button
          type="button"
          class="thumb"
          aria-label={t("sceneBrowser.backgroundPicker")}
          onclick={() => (pickerOpenFor = pickerOpenFor === scene.id ? null : scene.id)}
        >
          {#if bgOf(scene)}
            <img src={ctx.assets.url(bgOf(scene)!)} alt="" />
          {:else}
            <span class="placeholder" aria-hidden="true">🗺️</span>
          {/if}
        </button>
        {#if pickerOpenFor === scene.id}
          {@render assetPicker(bgOf(scene), (id) => setBackground(scene, id))}
        {/if}
        <span class="label">
          {t("sceneBrowser.sceneLabel", { n: i + 1 })}
          {#if scene.id === activeSceneId}<span class="badge">{t("sceneBrowser.activeBadge")}</span>{/if}
          {#if scene.id === viewedSceneId && scene.id !== activeSceneId}<span class="badge">{t("sceneBrowser.viewingBadge")}</span>{/if}
        </span>
        <div class="actions">
          <button type="button" onclick={() => activate(scene.id)} disabled={!ws || scene.id === activeSceneId}>{t("sceneBrowser.activate")}</button>
          <button type="button" onclick={() => view(scene.id)}>{t("sceneBrowser.view")}</button>
          <button type="button" onclick={() => configure(scene.id)}>{t("sceneBrowser.configure")}</button>
        </div>
      </li>
    {/each}
  </ul>
  <button type="button" class="create" onclick={create}>{t("sceneBrowser.create")}</button>
</section>

{#snippet assetPicker(selected: string | null, onPick: (id: string | null) => void)}
  <div class="picker">
    <button type="button" class="clear" onclick={() => onPick(null)}>{t("sceneBrowser.backgroundClear")}</button>
    {#each assetList as a (a.id)}
      <button type="button" class:selected={selected === a.id} title={a.original_name} onclick={() => onPick(a.id)}>
        <img src={ctx.assets.url(a.id)} alt={a.original_name} />
      </button>
    {/each}
  </div>
{/snippet}

<style lang="scss">
  .scene-browser {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
  }
  .list li.viewed {
    border-color: var(--accent);
  }
  .thumb {
    width: 48px;
    height: 48px;
    flex: none;
    border: none;
    border-radius: var(--radius-1);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-base);
    padding: 0;
    cursor: pointer;
  }
  .thumb:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .thumb img {
    width: 48px;
    height: 48px;
    object-fit: cover;
    display: block;
  }
  .picker {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .picker button {
    padding: 0;
    border: 2px solid transparent;
    border-radius: var(--radius-1);
    background: none;
    cursor: pointer;
  }
  .picker button.selected {
    border-color: var(--accent);
  }
  .picker button.clear {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    color: var(--text-primary);
    min-height: 44px;
  }
  .picker img {
    width: 48px;
    height: 48px;
    object-fit: cover;
    display: block;
  }
  .label {
    flex: 1 1 auto;
    color: var(--text-primary);
  }
  .badge {
    margin-left: var(--space-1);
    padding: 0 var(--space-1);
    border-radius: var(--radius-1);
    background: var(--accent);
    color: var(--on-accent);
    font-size: 0.75em;
  }
  .actions {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .actions button,
  .create,
  .hint button {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .actions button:focus-visible,
  .create:focus-visible,
  .hint button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
