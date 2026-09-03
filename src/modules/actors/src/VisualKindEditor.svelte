<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { type TokenVisual, type RenderVisual, type FaceVisual, type AnimatedSource, type Condition, type GeneratedCrop } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let {
    conditionOptions,
    onBuild,
    initial,
  }: {
    /** `[id, Condition]` pairs from the world's condition registry, populating the face-map
     * editor's condition dropdown. */
    conditionOptions: [string, Condition][];
    /** Called on every editor-state change with the currently buildable `TokenVisual`, or
     * `null` while the active kind's data is incomplete (see `buildVisual`). */
    onBuild: (visual: TokenVisual | null) => void;
    /** Post-create editing: when provided, every editor state initializes FROM this visual
     * (kind plus all per-kind fields, including a `"generated"` arm's art/crop/border/
     * background) instead of the blank create-time defaults. Read once at mount — a host that
     * switches edit targets remounts the editor (`{#key}`) rather than updating this prop. */
    initial?: TokenVisual;
  } = $props();

  // Seeds from `initial` once at mount — see the block comment above the other initial-seeded
  // states below; capturing the initial prop value is the intended semantics.
  // svelte-ignore state_referenced_locally
  let assetId = $state<string | null>(initial?.kind === "image" ? initial.asset : null);

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
   * and by `resetVisualEditor` for the top-level source, and inline for each new face row's and
   * each generated-art state's own `anim` — every call returns a distinct object, so no two
   * states ever share (alias) the same object.
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
    // `[...s.frames]` copies out of the reactive $state array: embedding it by reference would
    // leak a live Proxy into the built document, which `structuredClone` (the instanced-token
    // deep-copy path) cannot clone. Redundant with `ActorsPanel.create()`'s own deep
    // `$state.snapshot` at its only current call site, kept deliberately: this is a general
    // builder, and a future caller that reads its result without also snapshotting would
    // otherwise reintroduce the same leak.
    return s.sourceType === "frames"
      ? { type: "frames", frames: [...s.frames] }
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

  /**
   * The inverse projection of `animSourceToSource`: rebuilds a flat `AnimSourceState` from a wire
   * `AnimatedSource` (plus the `fps`/`loop` the source itself does not carry), for `initial`-driven
   * initialization. Unused sibling fields take `newAnimSourceState`'s defaults, so the returned
   * state is indistinguishable from one the user filled in by hand.
   * @param source The wire `AnimatedSource` to unfold.
   * @param fps The playback frames-per-second stored alongside the source.
   * @param loop Whether playback wraps at the end.
   * @returns A fresh `AnimSourceState` projecting back onto `source` via `animSourceToSource`.
   * @example
   * ```
   * // private helper; not part of the public API
   * animSourceStateFrom({ type: "frames", frames: ["a1"] }, 8, true);
   * ```
   */
  function animSourceStateFrom(source: AnimatedSource, fps: number, loop: boolean): AnimSourceState {
    return source.type === "frames"
      ? { sourceType: "frames", frames: [...source.frames], sheetAsset: null, rows: 1, cols: 1, count: null, fps, loop }
      : { sourceType: "sheet", frames: [], sheetAsset: source.asset, rows: source.rows, cols: source.cols, count: source.count, fps, loop };
  }

  /**
   * Unfolds a `"faces"`-kind `initial` visual's `faces` map into one editor row per face, each
   * with its OWN `anim` state (never shared — the same no-aliasing rule as `newAnimSourceState`).
   * A face whose visual is neither `"image"` nor `"animated"` — `FaceVisual` is `RenderVisual`,
   * whose wire type admits a `"generated"` face this editor's per-row image-or-animated machinery
   * cannot represent — fails closed to an INCOMPLETE placeholder row (no asset), so `buildVisual`
   * blocks the save rather than silently dropping that face's art.
   * @param faces The `faces` map of a `"faces"`-kind `initial` visual.
   * @returns One fresh `FaceRowState` per entry, keyed under the same names.
   * @example
   * ```
   * // private helper; not part of the public API
   * faceRowStatesFrom({ front: { kind: "image", asset: "a1" } });
   * ```
   */
  function faceRowStatesFrom(faces: Record<string, FaceVisual>): FaceRowState[] {
    return Object.entries(faces).map(([name, v]): FaceRowState => {
      if (v.kind === "image") return { name, kind: "image", asset: v.asset, anim: newAnimSourceState() };
      if (v.kind === "animated") return { name, kind: "animated", asset: null, anim: animSourceStateFrom(v.source, v.fps, v.loop) };
      return { name, kind: "image", asset: null, anim: newAnimSourceState() };
    });
  }

  /** Editor-local flat state for the `"generated"` kind — the framed art (image-or-animated, the
   * same shape as one face row's own fields) plus the crop and the optional border/background. */
  type GeneratedState = {
    /** Which of the state's own fields (`asset` vs `anim`) `buildVisual` projects into `art`. */
    artKind: "image" | "animated";
    /** The picked art asset id (`artKind: "image"` only). */
    asset: string | null;
    /** The art's own animated-source editor state (`artKind: "animated"` only). */
    anim: AnimSourceState;
    /** The shape the art is cropped to. */
    crop: GeneratedCrop;
    /** Whether a decorative border ring is emitted (`buildVisual` reads the color/width only
     * when this is set). */
    borderOn: boolean;
    /** Border ring color, a css `#rrggbb` string (`borderOn` only). */
    borderColor: string;
    /** Border ring width, in token-fraction px (`borderOn` only). */
    borderWidth: number;
    /** Whether a background fill is emitted (`buildVisual` reads the color only when set). */
    backgroundOn: boolean;
    /** Background fill color, a css `#rrggbb` string (`backgroundOn` only). */
    backgroundColor: string;
  };
  /**
   * Fresh, empty `GeneratedState` (circle crop, no art, border/background off but pre-filled with
   * sensible defaults so enabling one needs no further input). Every call returns a distinct
   * object — the same no-aliasing rule as `newAnimSourceState`.
   * @returns A new `GeneratedState` with no art selected.
   * @example
   * ```
   * // private helper; not part of the public API
   * newGeneratedState(); // { artKind: "image", asset: null, ..., crop: "circle", borderOn: false, ... }
   * ```
   */
  function newGeneratedState(): GeneratedState {
    return { artKind: "image", asset: null, anim: newAnimSourceState(), crop: "circle", borderOn: false, borderColor: "#ff8800", borderWidth: 0.06, backgroundOn: false, backgroundColor: "#102030" };
  }
  /**
   * Unfolds a `"generated"`-kind `initial` visual into its `GeneratedState`, for `initial`-driven
   * initialization. A garbled nested-`generated` `art` (the wire type cannot forbid it;
   * `resolveTokenVisual` fails closed on it) unfolds as an `"image"` art with NO asset — an
   * incomplete state that blocks the save rather than re-emitting a visual the renderer would
   * reject.
   * @param v The `"generated"`-kind `initial` visual to unfold.
   * @returns A fresh `GeneratedState` carrying `v`'s art/crop/border/background.
   * @example
   * ```
   * // private helper; not part of the public API
   * generatedStateFrom({ kind: "generated", art: { kind: "image", asset: "a1" }, crop: "circle", border: null, background: null });
   * ```
   */
  function generatedStateFrom(v: Extract<TokenVisual, { /** Narrows `TokenVisual` to its `"generated"` union member. */ kind: "generated" }>): GeneratedState {
    return {
      artKind: v.art.kind === "animated" ? "animated" : "image",
      asset: v.art.kind === "image" ? v.art.asset : null,
      anim: v.art.kind === "animated" ? animSourceStateFrom(v.art.source, v.art.fps, v.art.loop) : newAnimSourceState(),
      crop: v.crop,
      borderOn: v.border !== null,
      borderColor: v.border?.color ?? "#ff8800",
      borderWidth: v.border?.width ?? 0.06,
      backgroundOn: v.background !== null,
      backgroundColor: v.background?.color ?? "#102030",
    };
  }
  /**
   * The `"generated"` kind's art completeness check — the editor-side mirror of the acceptance
   * rule `resolveTokenVisual` enforces at the render boundary (`isValidGeneratedArt`):
   * an image art needs a picked asset; an animated art needs both a playable source
   * (`animSourceComplete`) AND a finite positive `fps` (`isValidAnimated`).
   * @param g The generated-kind editor state to check.
   * @returns `true` iff `g`'s art builds into a `RenderVisual` the renderer would accept.
   * @example
   * ```
   * // private helper; not part of the public API
   * generatedArtComplete(newGeneratedState()); // false
   * ```
   */
  function generatedArtComplete(g: GeneratedState): boolean {
    if (g.artKind === "image") return !!g.asset;
    return animSourceComplete(g.anim) && Number.isFinite(g.anim.fps) && g.anim.fps > 0;
  }

  // Every state below seeds from the `initial` prop ONCE at mount — the editor owns its state
  // afterward (a host switching edit targets remounts via `{#key}` instead), so capturing the
  // initial prop value is the intended semantics, per the prop's own doc.
  // svelte-ignore state_referenced_locally
  let visualKind = $state<"image" | "faces" | "animated" | "generated">(initial?.kind ?? "image");
  // svelte-ignore state_referenced_locally
  let topAnim = $state<AnimSourceState>(initial?.kind === "animated" ? animSourceStateFrom(initial.source, initial.fps, initial.loop) : newAnimSourceState());
  // svelte-ignore state_referenced_locally
  let faceRows = $state<FaceRowState[]>(initial?.kind === "faces" ? faceRowStatesFrom(initial.faces) : []);
  // svelte-ignore state_referenced_locally
  let defaultFace = $state(initial?.kind === "faces" ? initial.default : "");
  // svelte-ignore state_referenced_locally
  let faceMapRows = $state<{
    /** The condition registry id this row maps from; `""` when unset. */
    conditionId: string;
    /** The face name this row maps to; `""` when unset, or stale if the named face was since
     * renamed/removed (dropped by `buildVisual`, not fatal). */
    faceName: string;
  }[]>(
    initial?.kind === "faces" && initial.faceMap
      ? Object.entries(initial.faceMap).map(([conditionId, faceName]) => ({ conditionId, faceName }))
      : [],
  );
  // svelte-ignore state_referenced_locally
  let gen = $state<GeneratedState>(initial?.kind === "generated" ? generatedStateFrom(initial) : newGeneratedState());

  /**
   * Builds the `TokenVisual` the host should save (via `onBuild`), or `null` when the current
   * `visualKind`'s data is incomplete — the host's submit button is disabled on `null`, so an
   * incomplete visual is never persisted.
   *
   * Each per-kind branch returns a **fresh object literal carrying only that kind's own
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
   * - `"generated"`: the art projected from `gen`'s image-or-animated fields (the face-row
   *   pattern one level down) plus the crop and the enabled-only border/background; `null` if
   *   `generatedArtComplete` rejects the art, or an enabled border's width fails the same
   *   finite-positive rule `resolveTokenVisual` enforces — so an emitted visual always passes the
   *   renderer's own acceptance.
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
    if (visualKind === "generated") {
      if (!generatedArtComplete(gen)) return null;
      if (gen.borderOn && (!Number.isFinite(gen.borderWidth) || gen.borderWidth <= 0)) return null;
      const art: RenderVisual =
        gen.artKind === "image"
          ? { kind: "image", asset: gen.asset ?? "" }
          : { kind: "animated", source: animSourceToSource(gen.anim), fps: gen.anim.fps, loop: gen.anim.loop };
      return {
        kind: "generated",
        art,
        crop: gen.crop,
        border: gen.borderOn ? { color: gen.borderColor, width: gen.borderWidth } : null,
        background: gen.backgroundOn ? { color: gen.backgroundColor } : null,
      };
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
   * Clears every editor `$state` back to the blank create-time defaults (kind `"image"`, no
   * asset, no top-level anim source, no face rows, no generated-art state) — NOT back to the
   * `initial` prop's value: the only caller is the create form's post-create reset, where no
   * `initial` is passed. Called only by the exported `reset()` below.
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
    gen = newGeneratedState();
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
    <option value="generated">{t("actors.visualKindGenerated")}</option>
  </select>
</label>

{#snippet assetPicker(selected: string | null, onPick: (id: string) => void)}
  <div class="picker">
    {#if selected}
      <img class="current" src={ctx.assets.url(selected)} alt="" />
    {/if}
    <button
      type="button"
      data-testid="visual-pick"
      onclick={() =>
        void ctx.pickAsset({ kind: "image" }).then((id) => {
          if (id) onPick(id);
        })}
    >{t("actors.pickAsset")}</button>
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
    <button
      type="button"
      data-testid="visual-pick-frames"
      onclick={() =>
        void ctx.pickAsset({ kind: "image", multiple: true }).then((ids) => {
          if (ids) anim.frames = ids;
        })}
    >{t("actors.pickFrames")}</button>
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
{:else if visualKind === "generated"}
  <div class="generated-editor">
    <!-- The art sub-editor is the face-row pattern one level down: an image-or-animated kind
         select gating the same `assetPicker`/`animatedEditor` snippets each face row nests. -->
    <label>{t("actors.genArt")}
      <select bind:value={gen.artKind} aria-label={t("actors.genArt")}>
        <option value="image">{t("actors.visualKindImage")}</option>
        <option value="animated">{t("actors.visualKindAnimated")}</option>
      </select>
    </label>
    {#if gen.artKind === "image"}
      {@render assetPicker(gen.asset, (id: string) => (gen.asset = id))}
    {:else}
      {@render animatedEditor(gen.anim)}
    {/if}
    <label>{t("actors.genCrop")}
      <select bind:value={gen.crop} aria-label={t("actors.genCrop")}>
        <option value="circle">{t("actors.shapeCircle")}</option>
        <option value="square">{t("actors.shapeSquare")}</option>
      </select>
    </label>
    <label><input type="checkbox" bind:checked={gen.borderOn} /> {t("actors.genBorder")}</label>
    {#if gen.borderOn}
      <label>{t("actors.genBorderColor")}
        <input type="color" aria-label={t("actors.genBorderColor")} value={gen.borderColor} onchange={(e) => (gen.borderColor = e.currentTarget.value)} oninput={(e) => (gen.borderColor = e.currentTarget.value)} />
      </label>
      <!-- value + onchange/oninput (not bind:value): same fireEvent.change test-sync reason as the animated source's numeric inputs above. -->
      <label>{t("actors.genBorderWidth")}
        <input type="number" min="0.01" step="0.01" aria-label={t("actors.genBorderWidth")} value={gen.borderWidth} onchange={(e) => (gen.borderWidth = Number(e.currentTarget.value))} oninput={(e) => (gen.borderWidth = Number(e.currentTarget.value))} />
      </label>
    {/if}
    <label><input type="checkbox" bind:checked={gen.backgroundOn} /> {t("actors.genBackground")}</label>
    {#if gen.backgroundOn}
      <label>{t("actors.genBackgroundColor")}
        <input type="color" aria-label={t("actors.genBackgroundColor")} value={gen.backgroundColor} onchange={(e) => (gen.backgroundColor = e.currentTarget.value)} oninput={(e) => (gen.backgroundColor = e.currentTarget.value)} />
      </label>
    {/if}
  </div>
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
    color: var(--text-muted);
    font-size: 0.85em;
  }
  .picker {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .picker button {
    min-height: 2rem;
    cursor: pointer;
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
