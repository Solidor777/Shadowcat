<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, resolveTokenActor, type ActorSystem, type WireDocument, type FactionRegistrySystem, type Faction, type TokenVisual, type FaceVisual, type AnimatedSource, type ConditionRegistrySystem, type Condition } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the document store (same bridge as Surface.svelte): reading
  // `subscribe()` inside the derived registers a dependency so the list re-renders on create.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actorDocs = $derived.by(() => {
    subscribe();
    return ctx.documents.query("actor");
  });

  let name = $state("");
  let displayName = $state("");
  let assetId = $state<string | null>(null);
  let instanceOnDrop = $state(true);
  let hideName = $state(false);
  let faction = $state<string | null>(null);
  let shape = $state<"square" | "circle">("square");
  let sizeW = $state(1);
  let sizeH = $state(1);
  let darkvision = $state(0);
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
      : { type: "sheet", asset: s.sheetAsset ?? "", rows: s.rows, cols: s.cols, ...(s.count ? { count: s.count } : {}) };
  }

  type FaceRowState = { name: string; kind: "image" | "animated"; asset: string | null; anim: AnimSourceState };
  function faceRowToVisual(f: FaceRowState): FaceVisual {
    return f.kind === "image" ? { kind: "image", asset: f.asset ?? "" } : { kind: "animated", source: animSourceToSource(f.anim), fps: f.anim.fps, loop: f.anim.loop };
  }
  // Same completeness check the top-level image/animated branches apply to themselves,
  // applied per-row: an incomplete row (no asset picked / no frames or sheet asset)
  // must block the whole faces visual, not silently persist an empty asset/frame set.
  function faceRowComplete(f: FaceRowState): boolean {
    if (f.kind === "image") return !!f.asset;
    return (f.anim.sourceType === "frames" && f.anim.frames.length > 0) || (f.anim.sourceType === "sheet" && !!f.anim.sheetAsset);
  }

  let visualKind = $state<"image" | "faces" | "animated">("image");
  let topAnim = $state<AnimSourceState>(newAnimSourceState());
  let faceRows = $state<FaceRowState[]>([]);
  let defaultFace = $state("");
  let faceMapRows = $state<{ conditionId: string; faceName: string }[]>([]);

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.system as ConditionRegistrySystem | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  /** The single selected token, if any — drives the per-token face-swap palette below. */
  const selectedFaceToken = $derived.by((): WireDocument | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size === 0) return null;
    return ctx.documents.query("token").find((t) => ids.has(t.id)) ?? null;
  });

  /** The actor's declared faces map, if the selected token's actor visual is `faces` — drives
   * whether the palette shows at all (a plain image/animated token has nothing to swap). Routed
   * through `resolveTokenActor`, the single read-through every token-decoration consumer uses,
   * so a per-token `overrides.visual` is honored rather than bypassed. */
  const selectedFaceNames = $derived.by((): string[] => {
    subscribe();
    const tok = selectedFaceToken;
    if (!tok) return [];
    const eff = resolveTokenActor(tok, ctx.documents);
    return eff?.visual.kind === "faces" ? Object.keys(eff.visual.faces) : [];
  });

  /** Reads the RAW currently-stored face (never a resolved/defaulted value) — this is the
   * required `old` for the `/system/face` Update below; the server's field-level optimistic-
   * concurrency check rejects any Update whose `old` doesn't match the actual stored value. */
  function currentFace(tok: WireDocument): string | null {
    return (tok.system as { face?: string } | undefined)?.face ?? null;
  }

  function swapFace(faceName: string): void {
    const tok = selectedFaceToken;
    if (!tok || !ctx.canEdit(tok, "/system/face")) return;
    const old = currentFace(tok);
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/system/face", old, new: faceName }] }]);
  }

  function buildVisual(): TokenVisual | null {
    if (visualKind === "image") return assetId ? { kind: "image", asset: assetId } : null;
    if (visualKind === "animated") {
      if (topAnim.sourceType === "frames" && topAnim.frames.length === 0) return null;
      if (topAnim.sourceType === "sheet" && !topAnim.sheetAsset) return null;
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
    const faceMap = mapped.length > 0 ? Object.fromEntries(mapped.map((r) => [r.conditionId, r.faceName])) : undefined;
    return { kind: "faces", faces, default: defaultFace, ...(faceMap ? { faceMap } : {}) };
  }

  function resetVisualEditor(): void {
    visualKind = "image";
    topAnim = newAnimSourceState();
    faceRows = [];
    defaultFace = "";
    faceMapRows = [];
  }

  const factionOptions = $derived.by((): [string, Faction][] => {
    subscribe();
    const reg = ctx.documents.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
    return Object.entries(reg?.factions ?? {});
  });

  const isHidden = (a: WireDocument): boolean => a.permissions.property_overrides["/system/name"] === "owner_or_gm";

  function toggleHidden(a: WireDocument): void {
    const cur = a.permissions.property_overrides;
    const next = { ...cur };
    if (next["/system/name"] === "owner_or_gm") delete next["/system/name"];
    else next["/system/name"] = "owner_or_gm";
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/permissions/property_overrides", old: cur, new: next }] }]);
  }

  function refreshAssets(): void {
    void listAssets(ctx.world).then((a) => (assetList = a.filter((x) => x.content_type.startsWith("image/"))));
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  function create(): void {
    const visual = buildVisual();
    if (!name || !visual) return;
    const system: ActorSystem = {
      name,
      displayName: displayName || name,
      visual,
      size: { w: sizeW, h: sizeH },
      shape,
      faction,
      conditions: [],
      prototype: instanceOnDrop,
      ...(darkvision > 0 ? { vision: [{ mode: "darkvision" as const, range: darkvision }] } : {}),
    };
    const doc = buildActorDoc(ctx.world, system);
    if (hideName) setNameHidden(doc, true);
    ctx.dispatchIntent([{ op: "create", doc }]);
    name = "";
    displayName = "";
    assetId = null;
    hideName = false;
    faction = null;
    shape = "square";
    sizeW = 1;
    sizeH = 1;
    darkvision = 0;
    resetVisualEditor();
  }
</script>

<section class="actors">
  <h3>{t("actors.title")}</h3>
  {#if selectedFaceToken && selectedFaceNames.length > 0}
    <p class="hint">{t("actors.faceSwapHint")}</p>
    <div class="face-palette">
      {#each selectedFaceNames as name (name)}
        <button type="button" class:active={currentFace(selectedFaceToken) === name} onclick={() => swapFace(name)}>{name}</button>
      {/each}
    </div>
  {/if}
  <ul class="list">
    {#each actorDocs as a (a.id)}
      <li>
        <button
          type="button"
          class:selected={ctx.actorSelection.selectedId === a.id}
          onclick={() => ctx.actorSelection.select(a.id)}
        >{actorDisplayName(a.system as { name?: string; displayName?: string })}</button>
        {#if ctx.role === "gm"}
          <button type="button" class="hide-toggle" onclick={() => toggleHidden(a)}>
            {isHidden(a) ? t("actors.nameShown") : t("actors.hideName")}
          </button>
          <select
            aria-label={t("actors.faction")}
            value={(a.system as { faction?: string | null }).faction ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/faction", old: (a.system as { faction?: string | null }).faction ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">—</option>
            {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
          </select>
          <select
            aria-label={t("actors.shape")}
            value={(a.system as { shape?: string }).shape ?? "square"}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/shape", old: (a.system as { shape?: string }).shape ?? "square", new: e.currentTarget.value }] }])}
          >
            <option value="square">{t("actors.shapeSquare")}</option>
            <option value="circle">{t("actors.shapeCircle")}</option>
          </select>
          <!-- Per-row size inputs dispatch an update op (not bind:value), so e.currentTarget.value
               is a string; Number(...) coerces it to keep system.size numeric for actor.size × cell math. -->
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.width")}
            value={(a.system as { size?: { w: number } }).size?.w ?? 1}
            onchange={(e) => { const sz = (a.system as { size?: { w: number; h: number } }).size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/size", old: sz, new: { w: Number(e.currentTarget.value), h: sz.h } }] }]); }}
          />
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.height")}
            value={(a.system as { size?: { h: number } }).size?.h ?? 1}
            onchange={(e) => { const sz = (a.system as { size?: { w: number; h: number } }).size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/size", old: sz, new: { w: sz.w, h: Number(e.currentTarget.value) } }] }]); }}
          />
          <!-- Per-row darkvision input dispatches an update to /system/vision; range=0 clears to empty array. -->
          <input
            type="number" min="0" step="1" class="size-edit" aria-label={t("actors.darkvision")}
            value={(a.system as { vision?: Array<{ mode: string; range: number }> }).vision?.find((v) => v.mode === "darkvision")?.range ?? 0}
            onchange={(e) => { const range = Number(e.currentTarget.value); const cur = (a.system as { vision?: { mode: string; range: number }[] }).vision ?? null; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/vision", old: cur, new: range > 0 ? [{ mode: "darkvision", range }] : [] }] }]); }}
          />
        {/if}
      </li>
    {/each}
  </ul>
  <label class="keep">
    <input
      type="checkbox"
      checked={ctx.actorSelection.keepAfterPlace}
      onchange={(e) => ctx.actorSelection.setKeepAfterPlace(e.currentTarget.checked)}
    />
    {t("actors.keepAfterPlace")}
  </label>
  <form onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input placeholder={t("actors.name")} aria-label={t("actors.name")} bind:value={name} />
    <input placeholder={t("actors.displayName")} aria-label={t("actors.displayName")} bind:value={displayName} />
    <label><input type="checkbox" bind:checked={instanceOnDrop} /> {t("actors.instanceOnDrop")}</label>
    <label><input type="checkbox" bind:checked={hideName} /> {t("actors.hideName")}</label>
    <label>{t("actors.faction")}
      <select bind:value={faction}>
        <option value={null}>—</option>
        {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
      </select>
    </label>
    <label>{t("actors.shape")}
      <select bind:value={shape}>
        <option value="square">{t("actors.shapeSquare")}</option>
        <option value="circle">{t("actors.shapeCircle")}</option>
      </select>
    </label>
    <label>{t("actors.size")}
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.width")} bind:value={sizeW} />
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.height")} bind:value={sizeH} />
    </label>
    <label>
      {t("actors.darkvision")}
      <!-- value + onchange (not bind:value): bind:value on a number input reacts only to input events; the explicit handlers update state on change too. -->
      <input type="number" min="0" step="1" aria-label={t("actors.darkvision")} value={darkvision} onchange={(e) => (darkvision = Number(e.currentTarget.value))} oninput={(e) => (darkvision = Number(e.currentTarget.value))} />
    </label>
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
      <!-- value + onchange/oninput (not bind:value): bind:value on a number input reacts only to input events (see the darkvision input above); explicit handlers keep this in sync with `fireEvent.change` in tests too. -->
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
    <button type="submit" disabled={!name || !buildVisual()}>{t("actors.create")}</button>
  </form>
</section>

<style lang="scss">
  .actors {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
  .face-palette {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .face-palette button {
    min-width: 44px;
    min-height: 44px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .face-palette button.active {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list button {
    min-height: 44px;
    width: 100%;
    text-align: left;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .list button.selected {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
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
  label,
  button[type="submit"] {
    min-height: 32px;
  }
</style>
