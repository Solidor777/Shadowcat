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
    /** `[id, Condition]` pairs from the world's condition registry, populating the face-map
     * editor's condition dropdown. */
    conditionOptions: [string, Condition][];
    /** Called on every editor-state change with the currently buildable `TokenVisual`, or
     * `null` while the active kind's data is incomplete (see `buildVisual`). */
    onBuild: (visual: TokenVisual | null) => void;
  } = $props();

  let assetId = $state<string | null>(null);
  let assetList = $state<Asset[]>([]);

  /** Editor-local flat state for one `AnimatedSource`; `animSourceToSource` projects it into
   * the wire union, keeping only the fields for the active `sourceType`. */
  type AnimSourceState = {
    /** Which wire variant this source builds into; gates which other fields are read. */
    sourceType: "frames" | "sheet";
    /** Picked frame asset ids, in playback order (`sourceType: "frames"` only). */
    frames: string[];
    /** The sprite-sheet asset id, or `null` until picked (`sourceType: "sheet"` only). */
    sheetAsset: string | null;
    /** Sprite-sheet row count (`sourceType: "sheet"` only). */
    rows: number;
    /** Sprite-sheet column count (`sourceType: "sheet"` only). */
    cols: number;
    /** Sprite-sheet frame count override, or `null` to use `rows * cols`
     * (`sourceType: "sheet"` only). */
    count: number | null;
    /** Playback frames-per-second. */
    fps: number;
    /** Whether playback wraps at the end instead of holding the final frame. */
    loop: boolean;
  };
  /**
   * Fresh, empty `AnimSourceState` (frames mode, no frames picked). Called once at module init
   * and by `resetVisualEditor` for the top-level source, and inline for each new face row's own
   * `anim` — every call returns a distinct object, so no two rows and no row/top-level pair ever
   * share (alias) the same state.
   * @returns A new `AnimSourceState` with no frames or sheet asset selected.
   * @example
   * ```
   * // private helper; not part of the public API
   * newAnimSourceState(); // { sourceType: "frames", frames: [], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true }
   * ```
   */
  function newAnimSourceState(): AnimSourceState {
    return { sourceType: "frames", frames: [], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true };
  }
  /**
   * Projects the editor's flat `AnimSourceState` into the wire `AnimatedSource` union — the two
   * branches are mutually exclusive (a `"frames"` result carries no sheet fields, a `"sheet"`
   * result carries no `frames` array), mirroring `AnimatedSource`'s tagged-enum shape server-side.
   * @param s The editor's animated-source state to project.
   * @returns The `AnimatedSource` value to embed in a `RenderVisual`/`FaceVisual`.
   * @example
   * ```
   * // private helper; not part of the public API
   * animSourceToSource({ sourceType: "frames", frames: ["a1"], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true });
   * ```
   */
  function animSourceToSource(s: AnimSourceState): AnimatedSource {
    return s.sourceType === "frames"
      ? { type: "frames", frames: s.frames }
      : { type: "sheet", asset: s.sheetAsset ?? "", rows: s.rows, cols: s.cols, count: s.count };
  }

  /**
   * Shared "frames-nonempty AND sheet-asset-present" completeness check for an animated source,
   * used both per-face-row and for the top-level animated kind — an incomplete source (no frames
   * picked / no sheet asset) must block the whole visual, not silently persist an empty one.
   * @param anim The animated-source editor state to check.
   * @returns `true` iff `anim` has enough data to build a valid `AnimatedSource`.
   * @example
   * ```
   * // private helper; not part of the public API
   * animSourceComplete({ sourceType: "frames", frames: [], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true }); // false
   * ```
   */
  function animSourceComplete(anim: AnimSourceState): boolean {
    return (anim.sourceType === "frames" && anim.frames.length > 0) || (anim.sourceType === "sheet" && !!anim.sheetAsset);
  }

  /** Editor-local state for one row of a `"faces"`-kind visual's face map. */
  type FaceRowState = {
    /** The face name this row is keyed under in the built `faces` map. */
    name: string;
    /** Which of the row's own fields (`asset` vs `anim`) `faceRowToVisual` projects. */
    kind: "image" | "animated";
    /** The picked image asset id (`kind: "image"` only). */
    asset: string | null;
    /** The row's own animated-source editor state (`kind: "animated"` only). */
    anim: AnimSourceState;
  };
  /**
   * Projects one face-row's editor state into the `FaceVisual` stored under its name in the
   * built `faces` map — the face-row-scoped mirror of `buildVisual`'s image/animated branches.
   * An `"image"` row's `anim` state and an `"animated"` row's `asset` field are never read here
   * (only `f.kind` decides which literal is built), so switching a row's own `kind` can never
   * leak a sibling field into the emitted value: `FaceVisual` is `RenderVisual`,
   * a Rust internally-tagged enum whose variants
   * cannot carry each other's fields.
   * @param f The face-row editor state to project.
   * @returns The `FaceVisual` to store under this row's name.
   * @example
   * ```
   * // private helper; not part of the public API
   * faceRowToVisual({ name: "front", kind: "image", asset: "a1", anim: newAnimSourceState() });
   * ```
   */
  function faceRowToVisual(f: FaceRowState): FaceVisual {
    return f.kind === "image" ? { kind: "image", asset: f.asset ?? "" } : { kind: "animated", source: animSourceToSource(f.anim), fps: f.anim.fps, loop: f.anim.loop };
  }
  /**
   * Whether a face row has enough data to be included in a save — an `"image"` row needs a
   * picked `asset`; an `"animated"` row defers to `animSourceComplete` on its own `anim` state.
   * Read by `buildVisual`'s faces branch, which nulls the WHOLE visual (not just this row) if any
   * row fails this check.
   * @param f The face-row editor state to check.
   * @returns `true` iff this row has enough data for its current `kind` to be saved.
   * @example
   * ```
   * // private helper; not part of the public API
   * faceRowComplete({ name: "front", kind: "image", asset: "a1", anim: newAnimSourceState() }); // true
   * ```
   */
  function faceRowComplete(f: FaceRowState): boolean {
    return f.kind === "image" ? !!f.asset : animSourceComplete(f.anim);
  }

  let visualKind = $state<"image" | "faces" | "animated">("image");
  let topAnim = $state<AnimSourceState>(newAnimSourceState());
  let faceRows = $state<FaceRowState[]>([]);
  let defaultFace = $state("");
  let faceMapRows = $state<{
    /** The condition registry id this row maps from; `""` when unset. */
    conditionId: string;
    /** The face name this row maps to; `""` when unset, or stale if the named face was since
     * renamed/removed (dropped by `buildVisual`, not fatal). */
    faceName: string;
  }[]>([]);

  /**
   * Builds the `TokenVisual` the host should save (via `onBuild`), or `null` when the current
   * `visualKind`'s data is incomplete — the host's submit button is disabled on `null`, so an
   * incomplete visual is never persisted.
   *
   * Each of the three branches returns a **fresh object literal carrying only that kind's own
   * fields**, never a mutated copy of a previous kind's result — so switching `visualKind` (or a
   * face row's own `kind`, via `faceRowToVisual` above) can never leave a stale sibling field from
   * the PREVIOUS kind in the emitted value: an `"image"` result has no `faces`/`source` field to
   * go stale, an `"animated"` result has no `asset`/`faces` field, and a `"faces"` result's own
   * per-face entries have the same one-literal-per-kind property. This mirrors `TokenVisual`'s
   * wire shape — a Rust internally-tagged enum whose
   * variants cannot carry each other's fields, so there is no representable "stale sibling" state
   * on either side of the wire.
   * - `"image"`: `assetId` alone; `null` if nothing is picked.
   * - `"animated"`: the top-level `AnimSourceState` projected via `animSourceToSource`; `null` if
   *   `animSourceComplete` says the source is incomplete.
   * - `"faces"`: every `faceRows` entry projected via `faceRowToVisual`, keyed by name; `null` if
   *   there are zero rows, a blank or duplicate name, an incomplete row (`faceRowComplete`), or
   *   `defaultFace` no longer names a current row. A stale `faceMapRows` entry (naming a
   *   since-renamed/removed face) is DROPPED from the built `faceMap` rather than nulling the
   *   whole visual — the one recoverable case among these.
   * @returns The `TokenVisual` to report to the host, or `null` if the current kind's data is
   * incomplete.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the `$effect` below on every
   * // relevant state change, not called directly
   * buildVisual(); // { kind: "image", asset: "a1" } | null
   * ```
   */
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

  /**
   * Clears every editor `$state` back to its initial value (kind `"image"`, no asset, no
   * top-level anim source, no face rows). Called only by the exported `reset()` below.
   * @returns Nothing; mutates the component's own `$state` fields.
   * @example
   * ```
   * // private helper; not part of the public API — invoked only by the exported `reset()`
   * resetVisualEditor();
   * ```
   */
  function resetVisualEditor(): void {
    visualKind = "image";
    topAnim = newAnimSourceState();
    faceRows = [];
    defaultFace = "";
    faceMapRows = [];
    assetId = null;
  }

  /**
   * Instance export: the host resets the editor after a successful create, via
   * `bind:this={visualEditor}` in `ActorsPanel` (`visualEditor?.reset()`).
   * @returns Nothing; delegates to `resetVisualEditor`.
   * @example
   * ```
   * // public instance method; called by the host through `bind:this`
   * declare const visualEditor: { reset(): void };
   * visualEditor.reset();
   * ```
   */
  export function reset(): void {
    resetVisualEditor();
  }

  /**
   * Refetches the world's image assets (filtered to `image/*` content types) into `assetList`,
   * feeding every `assetPicker` snippet instance. Called once at mount and on every
   * `AssetChanged` broadcast, via the `$effect` below.
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
      // Every record here carries the true, current version — reconciling on each load
      // self-heals a uuid whose cache-bust state went stale from a missed AssetChanged frame.
      ctx.assets.reconcile(a);
    });
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  // Continuously report the current built visual (or null when incomplete) to the host, which
  // gates its submit button and consumes it at create time. buildVisual reads every editor
  // $state, so this effect re-emits on any change, keeping `onBuild` synced with every field.
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
