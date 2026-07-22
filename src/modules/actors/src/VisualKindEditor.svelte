<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { listAssets, type TokenVisual, type FaceVisual, type AnimatedSource, type Condition } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let {
    conditionOptions,
    onBuild,
  }: {
    conditionOptions: [string, Condition][];
    onBuild: (visual: TokenVisual | null) => void;
  } = $props();

  let assetId = $state<string | null>(null);
  let assetList = $state<Asset[]>([]);

  type AnimSourceState = {
    sourceType: "frames" | "sheet";
    frames: string[];
    sheetAsset: string | null;
    rows: number;
    cols: number;
    count: number | null;
    fps: number;
    loop: boolean;
  };
  function newAnimSourceState(): AnimSourceState {
    return { sourceType: "frames", frames: [], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true };
  }
  function animSourceToSource(s: AnimSourceState): AnimatedSource {
    return s.sourceType === "frames"
      ? { type: "frames", frames: s.frames }
      : { type: "sheet", asset: s.sheetAsset ?? "", rows: s.rows, cols: s.cols, count: s.count };
  }

  // Shared "frames-nonempty AND sheet-asset-present" completeness check for an animated source,
  // used both per-face-row and for the top-level animated kind — an incomplete source (no frames
  // picked / no sheet asset) must block the whole visual, not silently persist an empty one.
  function animSourceComplete(anim: AnimSourceState): boolean {
    return (anim.sourceType === "frames" && anim.frames.length > 0) || (anim.sourceType === "sheet" && !!anim.sheetAsset);
  }

  type FaceRowState = { name: string; kind: "image" | "animated"; asset: string | null; anim: AnimSourceState };
  function faceRowToVisual(f: FaceRowState): FaceVisual {
    return f.kind === "image" ? { kind: "image", asset: f.asset ?? "" } : { kind: "animated", source: animSourceToSource(f.anim), fps: f.anim.fps, loop: f.anim.loop };
  }
  function faceRowComplete(f: FaceRowState): boolean {
    return f.kind === "image" ? !!f.asset : animSourceComplete(f.anim);
  }

  let visualKind = $state<"image" | "faces" | "animated">("image");
  let topAnim = $state<AnimSourceState>(newAnimSourceState());
  let faceRows = $state<FaceRowState[]>([]);
  let defaultFace = $state("");
  let faceMapRows = $state<{ conditionId: string; faceName: string }[]>([]);

  function buildVisual(): TokenVisual | null {
    if (visualKind === "image") return assetId ? { kind: "image", asset: assetId } : null;
    if (visualKind === "animated") {
      if (!animSourceComplete(topAnim)) return null;
      return { kind: "animated", source: animSourceToSource(topAnim), fps: topAnim.fps, loop: topAnim.loop };
    }
    if (faceRows.length === 0 || !defaultFace || faceRows.some((f) => !f.name)) return null;
    const names = faceRows.map((f) => f.name);
    const uniqueNames = new Set(names);
    if (uniqueNames.size !== names.length) return null; // duplicate face names collapse silently otherwise
    if (faceRows.some((f) => !faceRowComplete(f))) return null;
    if (!uniqueNames.has(defaultFace)) return null; // defaultFace must reference a current row (no stale reference)
    const faces: Record<string, FaceVisual> = {};
    for (const f of faceRows) faces[f.name] = faceRowToVisual(f);
    // A stale faceMap row (faceName no longer present after a rename/removal) is dropped,
    // not fatal — recoverable, unlike a stale defaultFace which blocks the whole save.
    const mapped = faceMapRows.filter((r) => r.conditionId && r.faceName && uniqueNames.has(r.faceName));
    const faceMap = mapped.length > 0 ? Object.fromEntries(mapped.map((r) => [r.conditionId, r.faceName])) : null;
    return { kind: "faces", faces, default: defaultFace, faceMap };
  }

  function resetVisualEditor(): void {
    visualKind = "image";
    topAnim = newAnimSourceState();
    faceRows = [];
    defaultFace = "";
    faceMapRows = [];
    assetId = null;
  }

  /** Instance export: the host resets the editor after a successful create. */
  export function reset(): void {
    resetVisualEditor();
  }

  function refreshAssets(): void {
    void listAssets(ctx.world).then((a) => (assetList = a.filter((x) => x.content_type.startsWith("image/"))));
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  // Continuously report the current built visual (or null when incomplete) to the host, which
  // gates its submit button and consumes it at create time. buildVisual reads every editor
  // $state, so this effect re-emits on any change — mirroring the host's former inline read.
  $effect(() => {
    onBuild(buildVisual());
  });
</script>

<label>{t("actors.visualKind")}
  <select bind:value={visualKind} aria-label={t("actors.visualKind")}>
    <option value="image">{t("actors.visualKindImage")}</option>
    <option value="faces">{t("actors.visualKindFaces")}</option>
    <option value="animated">{t("actors.visualKindAnimated")}</option>
  </select>
</label>

{#snippet assetPicker(selected: string | null, onPick: (id: string) => void)}
  <div class="picker">
    {#each assetList as a (a.id)}
      <button type="button" class:selected={selected === a.id} title={a.original_name} onclick={() => onPick(a.id)}>
        <img src={ctx.assets.url(a.id)} alt={a.original_name} />
      </button>
    {/each}
  </div>
{/snippet}

{#snippet animatedEditor(anim: AnimSourceState)}
  <label>{t("actors.animSourceType")}
    <select bind:value={anim.sourceType}>
      <option value="frames">{t("actors.animSourceFrames")}</option>
      <option value="sheet">{t("actors.animSourceSheet")}</option>
    </select>
  </label>
  {#if anim.sourceType === "frames"}
    <p class="hint">{t("actors.animFramesHint")}</p>
    {@render assetPicker(null, (id: string) => (anim.frames = [...anim.frames, id]))}
    <ol class="frame-list">
      {#each anim.frames as f, i (i)}
        <li><img src={ctx.assets.url(f)} alt="" /> <button type="button" onclick={() => (anim.frames = anim.frames.filter((_: string, j: number) => j !== i))}>{t("actors.animRemoveFrame")}</button></li>
      {/each}
    </ol>
  {:else}
    {@render assetPicker(anim.sheetAsset, (id: string) => (anim.sheetAsset = id))}
    <label>{t("actors.animRows")} <input type="number" min="1" step="1" bind:value={anim.rows} /></label>
    <label>{t("actors.animCols")} <input type="number" min="1" step="1" bind:value={anim.cols} /></label>
    <label>{t("actors.animCount")} <input type="number" min="1" step="1" value={anim.count ?? ""} onchange={(e) => (anim.count = e.currentTarget.value ? Number(e.currentTarget.value) : null)} oninput={(e) => (anim.count = e.currentTarget.value ? Number(e.currentTarget.value) : null)} /></label>
  {/if}
  <!-- value + onchange/oninput (not bind:value): bind:value on a number input reacts only to input events; explicit handlers keep this in sync with `fireEvent.change` in tests too. -->
  <label>{t("actors.animFps")} <input type="number" min="1" step="1" aria-label={t("actors.animFps")} value={anim.fps} onchange={(e) => (anim.fps = Number(e.currentTarget.value))} oninput={(e) => (anim.fps = Number(e.currentTarget.value))} /></label>
  <label><input type="checkbox" bind:checked={anim.loop} /> {t("actors.animLoop")}</label>
{/snippet}

{#if visualKind === "image"}
  {@render assetPicker(assetId, (id: string) => (assetId = id))}
{:else if visualKind === "animated"}
  {@render animatedEditor(topAnim)}
{:else}
  <div class="faces-editor">
    {#each faceRows as f, i (i)}
      <div class="face-row">
        <input placeholder={t("actors.faceName")} aria-label={t("actors.faceName")} bind:value={f.name} />
        <select bind:value={f.kind} aria-label={t("actors.faceKind")}>
          <option value="image">{t("actors.visualKindImage")}</option>
          <option value="animated">{t("actors.visualKindAnimated")}</option>
        </select>
        {#if f.kind === "image"}
          {@render assetPicker(f.asset, (id: string) => (f.asset = id))}
        {:else}
          {@render animatedEditor(f.anim)}
        {/if}
        <button type="button" onclick={() => (faceRows = faceRows.filter((_, j) => j !== i))}>{t("actors.faceRemove")}</button>
      </div>
    {/each}
    <button type="button" onclick={() => (faceRows = [...faceRows, { name: "", kind: "image", asset: null, anim: newAnimSourceState() }])}>{t("actors.faceAdd")}</button>
    <label>{t("actors.faceDefault")}
      <select bind:value={defaultFace} aria-label={t("actors.faceDefault")}>
        <option value="">—</option>
        {#each faceRows as f, fi (fi)}<option value={f.name}>{f.name}</option>{/each}
      </select>
    </label>
    <div class="face-map-editor">
      <p class="hint">{t("actors.faceMapHint")}</p>
      {#each faceMapRows as r, i (i)}
        <div class="face-map-row">
          <select bind:value={r.conditionId} aria-label={t("actors.faceMapCondition")}>
            <option value="">—</option>
            {#each conditionOptions as [id, c] (id)}<option value={id}>{c.name}</option>{/each}
          </select>
          <select bind:value={r.faceName} aria-label={t("actors.faceMapFace")}>
            <option value="">—</option>
            {#each faceRows as f, fi (fi)}<option value={f.name}>{f.name}</option>{/each}
          </select>
          <button type="button" onclick={() => (faceMapRows = faceMapRows.filter((_, j) => j !== i))}>{t("actors.faceRemove")}</button>
        </div>
      {/each}
      <button type="button" onclick={() => (faceMapRows = [...faceMapRows, { conditionId: "", faceName: "" }])}>{t("actors.faceMapAdd")}</button>
    </div>
  </div>
{/if}

<style lang="scss">
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
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
  .picker img {
    width: 48px;
    height: 48px;
    object-fit: cover;
    display: block;
  }
  input,
  label {
    min-height: 32px;
  }
</style>
