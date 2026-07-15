<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildSceneDoc, type WireDocument, type WorldSettingsSystem, type SceneSystem } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

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
    return (ws?.system as WorldSettingsSystem | undefined)?.activeScene ?? null;
  });
  // The GM's own rendered scene (roam or followed active). Reading the getter tracks the doc store
  // (via subscribe above) + the session's gmViewedScene state.
  const viewedSceneId = $derived.by((): string | null => {
    subscribe();
    return ctx.viewedSceneId;
  });
  const roaming = $derived(viewedSceneId !== null && viewedSceneId !== activeSceneId);

  function bgOf(scene: WireDocument): string | null {
    return (scene.system as SceneSystem | undefined)?.background ?? null;
  }

  /** Set the scene players render. OCC pre-image is the REAL current activeScene (or null when
   * genuinely absent) — never a defaulted value. No-op with a debug hint if world-settings is
   * absent (game-settings seeds it on the same GM Welcome). */
  function activate(sceneId: string): void {
    if (!ws) return;
    const old = (ws.system as WorldSettingsSystem | undefined)?.activeScene ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: ws.id, changes: [{ path: "/system/activeScene", old, new: sceneId }] }]);
  }

  /** GM local roam (no effect on players). */
  function view(sceneId: string): void {
    ctx.setGmViewedScene(sceneId);
  }
  function followActive(): void {
    ctx.setGmViewedScene(null);
  }

  /** Deep-link the game-settings per-scene section to this scene. */
  function configure(sceneId: string): void {
    ctx.sceneSelection.select(sceneId);
    ctx.panels.open("game-settings");
  }

  function create(): void {
    ctx.dispatchIntent([{ op: "create", doc: buildSceneDoc(ctx.world) }]);
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
        <div class="thumb">
          {#if bgOf(scene)}
            <img src={ctx.assets.url(bgOf(scene)!)} alt="" />
          {:else}
            <span class="placeholder" aria-hidden="true">🗺️</span>
          {/if}
        </div>
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
    border-radius: var(--radius-1);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-base);
  }
  .thumb img {
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
