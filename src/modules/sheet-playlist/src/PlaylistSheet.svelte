<script lang="ts">
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { createSubscriber } from "svelte/reactivity";
  import { getPointer } from "@shadowcat/core";
  import type { PlaylistEngine, PlaylistMode, PlaylistTrack, AudioChannel } from "@shadowcat/core";
  import { addTrack, removeTrack, moveTrack, setTrack } from "./trackOps";

  // Playlist sheet: edits the playlist engine body. Every scalar field is one `setField`
  // call on `change`; the tracks editor replaces the WHOLE `tracks` array per mutation
  // (`set_pointer` cannot grow arrays — `trackOps` clones, never mutates the store's array).
  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The playlist document this sheet edits (a playlist is never embedded, never parented). */
    docId: string;
    /** The write root for the opaque `system` tree; `enginePrefix`/`namePrefix` derive from it. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to the header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive subscription bridge: calling subscribe() inside a derived registers a
  // dependency on the document store, so the sheet re-renders after its own confirmed
  // writes (and any other store change) — the sheets-skill invariant.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const enginePrefix = $derived(`${basePrefix}/engine`);
  const namePrefix = $derived(`${basePrefix}/name`);
  const tracksPath = $derived(`${enginePrefix}/tracks`);

  const doc = $derived.by(() => {
    subscribe();
    const d = ctx.documents.get(docId);
    return d?.doc_type === "playlist" ? d : undefined;
  });
  const engine = $derived.by((): PlaylistEngine | undefined =>
    doc ? (getPointer(doc, enginePrefix) as PlaylistEngine | undefined) : undefined,
  );
  const readOnly = $derived(!doc || !ctx.canEdit(doc, enginePrefix));

  /** Playback-mode options for the mode select (wire snake_case value → i18n label key). */
  const MODES: {
    /** The wire value. */
    value: PlaylistMode;
    /** The label's i18n key. */
    label: string;
  }[] = [
    { value: "sequential", label: "sheetPlaylist.modeSequential" },
    { value: "shuffle", label: "sheetPlaylist.modeShuffle" },
    { value: "loop_all", label: "sheetPlaylist.modeLoopAll" },
    { value: "single", label: "sheetPlaylist.modeSingle" },
  ];
  /** Mixer-channel options for the channel select. */
  const CHANNELS: {
    /** The wire value. */
    value: AudioChannel;
    /** The label's i18n key. */
    label: string;
  }[] = [
    { value: "music", label: "sheetPlaylist.channelMusic" },
    { value: "ambience", label: "sheetPlaylist.channelAmbience" },
    { value: "sfx", label: "sheetPlaylist.channelSfx" },
  ];

  /** Apply a partial edit to one track as a whole-array write.
   * @param i The track index.
   * @param patch The fields to change.
   * @example
   * ```
   * // private helper; exercised through `PlaylistSheet.test.ts`'s field-edit cases
   * ```
   */
  function patchTrack(i: number, patch: Partial<PlaylistTrack>): void {
    if (!engine) return;
    const next = setTrack(engine.tracks, i, { ...engine.tracks[i], ...patch });
    setField(ctx, docId, tracksPath, engine.tracks, next);
  }

  /** Open the asset picker scoped to the audio kind and assign the pick to the track.
   * @param i The track index.
   * @example
   * ```
   * // private handler; exercised through `PlaylistSheet.test.ts`'s picker case
   * ```
   */
  async function pickAsset(i: number): Promise<void> {
    const picked = await ctx.pickAsset({ kind: "audio" });
    if (typeof picked === "string") patchTrack(i, { asset: picked });
  }

  /** Open the asset picker and append a track for the pick. Pick-first because
   * `PlaylistEngine::validate` rejects a track with an empty asset id, so the sheet can never
   * stage an unassigned row — a cancelled pick simply adds nothing.
   * @example
   * ```
   * // private handler; exercised through `PlaylistSheet.test.ts`'s add cases
   * ```
   */
  async function addTrackPicked(): Promise<void> {
    const picked = await ctx.pickAsset({ kind: "audio" });
    if (typeof picked !== "string" || !engine) return;
    setField(ctx, docId, tracksPath, engine.tracks, addTrack(engine.tracks, picked));
  }
</script>

{#if doc && engine}
  <header>
    <input
      type="text"
      data-testid="playlist-name"
      aria-label={t("sheetPlaylist.name")}
      value={doc.name ?? ""}
      disabled={readOnly}
      onchange={(e) => setField(ctx, docId, namePrefix, doc.name ?? null, (e.currentTarget as HTMLInputElement).value)}
    />
    <button type="button" onclick={close}>✕</button>
  </header>

  <label>
    {t("sheetPlaylist.mode")}
    <select
      data-testid="playlist-mode"
      aria-label={t("sheetPlaylist.mode")}
      disabled={readOnly}
      value={engine.mode}
      onchange={(e) => setField(ctx, docId, `${enginePrefix}/mode`, engine.mode, (e.currentTarget as HTMLSelectElement).value)}
    >
      {#each MODES as m (m.value)}
        <option value={m.value} selected={engine.mode === m.value}>{t(m.label)}</option>
      {/each}
    </select>
  </label>

  <label>
    {t("sheetPlaylist.channel")}
    <select
      data-testid="playlist-channel"
      aria-label={t("sheetPlaylist.channel")}
      disabled={readOnly}
      value={engine.channel}
      onchange={(e) => setField(ctx, docId, `${enginePrefix}/channel`, engine.channel, (e.currentTarget as HTMLSelectElement).value)}
    >
      {#each CHANNELS as c (c.value)}
        <option value={c.value} selected={engine.channel === c.value}>{t(c.label)}</option>
      {/each}
    </select>
  </label>

  <label>
    {t("sheetPlaylist.fade")}
    <input
      type="number"
      min="0"
      max="10000"
      step="100"
      data-testid="playlist-fade"
      aria-label={t("sheetPlaylist.fade")}
      disabled={readOnly}
      value={engine.fadeMs}
      onchange={(e) => setField(ctx, docId, `${enginePrefix}/fadeMs`, engine.fadeMs, Number((e.currentTarget as HTMLInputElement).value))}
    />
  </label>

  <h4>{t("sheetPlaylist.tracks")}</h4>
  <ul data-testid="playlist-tracks">
    {#each engine.tracks as track, i (i)}
      <li data-testid="track-row" data-track-index={i}>
        <button
          type="button"
          data-testid="track-asset"
          aria-label={t("sheetPlaylist.trackAsset", { n: i + 1 })}
          disabled={readOnly}
          onclick={() => void pickAsset(i)}
        >
          {track.asset || "—"}
        </button>
        <button
          type="button"
          data-testid="track-preview"
          aria-label={t("sheetPlaylist.trackPreview", { n: i + 1 })}
          onclick={() => ctx.audio.playOneShot(track.asset)}
        >▶</button>
        <input
          type="text"
          data-testid="track-name"
          aria-label={t("sheetPlaylist.trackName", { n: i + 1 })}
          disabled={readOnly}
          value={track.name ?? ""}
          placeholder={track.asset}
          onchange={(e) => patchTrack(i, { name: (e.currentTarget as HTMLInputElement).value || null })}
        />
        <input
          type="number"
          min="0"
          max="1"
          step="0.05"
          data-testid="track-gain"
          aria-label={t("sheetPlaylist.trackGain", { n: i + 1 })}
          disabled={readOnly}
          value={track.gain}
          onchange={(e) => patchTrack(i, { gain: Number((e.currentTarget as HTMLInputElement).value) })}
        />
        <input
          type="checkbox"
          data-testid="track-loop"
          aria-label={t("sheetPlaylist.trackLoop", { n: i + 1 })}
          disabled={readOnly}
          checked={track.loop}
          onchange={(e) => patchTrack(i, { loop: (e.currentTarget as HTMLInputElement).checked })}
        />
        <button
          type="button"
          data-testid="track-up"
          aria-label={t("sheetPlaylist.trackUp", { n: i + 1 })}
          disabled={readOnly || i === 0}
          onclick={() => engine && setField(ctx, docId, tracksPath, engine.tracks, moveTrack(engine.tracks, i, -1))}
        >↑</button>
        <button
          type="button"
          data-testid="track-down"
          aria-label={t("sheetPlaylist.trackDown", { n: i + 1 })}
          disabled={readOnly || i === engine.tracks.length - 1}
          onclick={() => engine && setField(ctx, docId, tracksPath, engine.tracks, moveTrack(engine.tracks, i, 1))}
        >↓</button>
        <button
          type="button"
          data-testid="track-remove"
          aria-label={t("sheetPlaylist.trackRemove", { n: i + 1 })}
          disabled={readOnly}
          onclick={() => engine && setField(ctx, docId, tracksPath, engine.tracks, removeTrack(engine.tracks, i))}
        >✕</button>
      </li>
    {/each}
  </ul>
  {#if !readOnly}
    <button
      type="button"
      data-testid="playlist-add-track"
      onclick={() => void addTrackPicked()}
    >{t("sheetPlaylist.addTrack")}</button>
  {/if}
{:else}
  <p>{t("sheetPlaylist.missing")}</p>
{/if}
