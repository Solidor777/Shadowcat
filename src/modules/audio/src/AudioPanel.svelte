<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { createSubscriber } from "svelte/reactivity";
  import {
    buildPlaylistDoc,
    listAssets,
    AUDIO_STATE_DOC_TYPE,
    PLAYLIST_DOC_TYPE,
    type AudioChannelId,
    type AudioStateEngine,
    type PlayingTrack,
    type WireAudioOp,
    type WireDocument,
    type WireSearchHit,
    type SubscriptionHandle,
  } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;
  // Reactive subscription bridge: every derived below calls subscribe() before reading the
  // document store (the sheets-skill invariant — a plain read establishes no dependency).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const CHANNELS: AudioChannelId[] = ["master", "music", "ambience", "sfx", "ui"];
  const CHANNEL_LABEL: Record<AudioChannelId, string> = {
    master: "audio.channelMaster",
    music: "audio.channelMusic",
    ambience: "audio.channelAmbience",
    sfx: "audio.channelSfx",
    ui: "audio.channelUi",
  };

  const audioState = $derived.by((): AudioStateEngine | undefined => {
    subscribe();
    return ctx.documents.query(AUDIO_STATE_DOC_TYPE)[0]?.engine as AudioStateEngine | undefined;
  });
  /** Live (unpaused) playing entries — the ONLY DOM-visible playback signal a Playwright
   * spec can assert (it cannot inspect Web Audio internals). */
  const liveCount = $derived((audioState?.playing ?? []).filter((p) => p.pausedAt == null).length);

  // Asset durations for the seek slider's max (the asset listing is the only client surface
  // carrying durationMs).
  let durations = $state(new Map<string, number>());
  $effect(() => {
    let cancelled = false;
    void listAssets(ctx.world)
      .then((assets) => {
        if (cancelled) return;
        durations = new Map(
          assets
            .filter((a) => a.duration_ms != null)
            .map((a) => [a.id, Number(a.duration_ms)]),
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  });

  // Playlists live search — the ActorsPanel $effect shape (cancel guard, handle capture,
  // server-side docTypes filter as the ONE filter).
  let query = $state("");
  let hits = $state<WireDocument[]>([]);
  let newName = $state("");
  $effect(() => {
    const q = query.trim();
    if (!q) {
      hits = [];
      return;
    }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20, docTypes: [PLAYLIST_DOC_TYPE] }, (found: WireSearchHit[]) => {
        if (cancelled) return;
        hits = found.map((h) => h.document);
      })
      .then((h) => {
        if (cancelled) h.unsubscribe();
        else handle = h;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      handle?.unsubscribe();
    };
  });

  const canCreate = $derived(ctx.canCreate(PLAYLIST_DOC_TYPE));
  const isGm = $derived(ctx.role === "gm");
  // The GM listen-as picker's candidates: every token of the viewed scene (the
  // reactive-subscription bridge every other derived read in this panel uses).
  const sceneTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const scene = ctx.viewedSceneId;
    return ctx.documents.query("token").filter((d) => d.parent_id === scene);
  });

  /** The entry's current position in seconds (frozen at `pausedAt` when paused), clamped
   * non-negative.
   * @param p The playing entry.
   * @returns Seconds from the track start.
   * @example
   * ```
   * // private helper; rendered as the now-playing position readout
   * ```
   */
  function positionSecs(p: PlayingTrack): number {
    return Math.max(0, ((p.pausedAt ?? ctx.audio.serverNow()) - p.startedAt) / 1000);
  }

  /** The `m:ss` readout for a playing entry.
   * @param p The playing entry.
   * @returns The formatted position.
   * @example
   * ```
   * // private helper; rendered as the now-playing position readout
   * ```
   */
  function positionLabel(p: PlayingTrack): string {
    const secs = positionSecs(p);
    return `${Math.floor(secs / 60)}:${String(Math.floor(secs % 60)).padStart(2, "0")}`;
  }

  /** The now-playing row's display name — the raw asset id today (a playlist track's own
   * `name` rides the playlist document, which this panel does not join per row).
   * @param p The playing entry.
   * @returns The display string.
   * @example
   * ```
   * // private helper; rendered through the now-playing list
   * ```
   */
  function displayName(p: PlayingTrack): string {
    return p.asset;
  }

  /** Send a GM `play` transport op for a playlist, with every override absent.
   * @param id The playlist document id.
   * @example
   * ```
   * // private handler; exercised through `AudioPanel.test.ts`'s playlist-row cases
   * ```
   */
  function play(id: string): void {
    ctx.audio.transport({ type: "play", playlist: id, asset: null, track_index: null, channel: null, gain: null, loop: null });
  }

  /** Create a playlist document from the name field and dispatch it (owner = this user, so
   * the creator manages their own playlist), then open its sheet — creating a playlist takes
   * you straight to its track editor.
   * @example
   * ```
   * // private handler; exercised through `AudioPanel.test.ts`'s create case
   * ```
   */
  function createPlaylist(): void {
    const name = newName.trim();
    if (!name) return;
    const doc = buildPlaylistDoc(
      ctx.world,
      name,
      { tracks: [], mode: "sequential", channel: "music", fadeMs: 0 },
      { owner: ctx.selfId },
    );
    ctx.dispatchIntent([{ op: "create", doc }]);
    ctx.openDocument({ docId: doc.id });
    newName = "";
  }

  /** Dispatch a delete of a playlist document (gated on `ctx.canDelete` at render).
   * @param doc The playlist document to delete.
   * @example
   * ```
   * // private handler; exercised through `AudioPanel.test.ts`'s delete-gating case
   * ```
   */
  function deletePlaylist(doc: WireDocument): void {
    ctx.dispatchIntent([{ op: "delete", doc }]);
  }

  /** Send a row-scoped or global transport op.
   * @param op The transport op.
   * @example
   * ```
   * // private handler; exercised through `AudioPanel.test.ts`'s transport cases
   * ```
   */
  function playOp(op: WireAudioOp): void {
    ctx.audio.transport(op);
  }
</script>

<section class="audio-panel" data-testid="audio-panel" data-audio-playing={liveCount}>
  <h3>{t("audio.channelsTitle")}</h3>
  {#each CHANNELS as id (id)}
    <div class="channel-row">
      <label>
        {t(CHANNEL_LABEL[id])}
        <input
          type="range"
          min="0"
          max="1"
          step="0.05"
          data-testid={`channel-gain-${id}`}
          aria-label={t("audio.channelGainFor", { channel: t(CHANNEL_LABEL[id]) })}
          value={ctx.audio.channels[id].gain}
          onchange={(e) => ctx.audio.setChannel(id, { gain: Number((e.currentTarget as HTMLInputElement).value) })}
        />
      </label>
      <button
        type="button"
        data-testid={`channel-mute-${id}`}
        aria-label={t(ctx.audio.channels[id].muted ? "audio.unmuteChannel" : "audio.muteChannel", { channel: t(CHANNEL_LABEL[id]) })}
        onclick={() => ctx.audio.setChannel(id, { muted: !ctx.audio.channels[id].muted })}
      >{ctx.audio.channels[id].muted ? "🔇" : "🔊"}</button>
    </div>
  {/each}

  {#if isGm}
    <label>
      {t("audio.listenAs")}
      <select
        data-testid="audio-listen-as"
        aria-label={t("audio.listenAs")}
        onchange={(e) => ctx.audio.listenAs((e.currentTarget as HTMLSelectElement).value || null)}
      >
        <option value="">{t("audio.listenAsOwn")}</option>
        {#each sceneTokens as tok (tok.id)}
          <option value={tok.id}>{tok.name ?? tok.id}</option>
        {/each}
      </select>
    </label>
  {/if}

  <h3>{t("audio.nowPlaying")}</h3>  <ul class="now-playing">
    {#each audioState?.playing ?? [] as p (p.id)}
      <li data-testid="playing-row" data-playing-id={p.id}>
        <span class="playing-name">{displayName(p)}</span>
        <span class="playing-pos">{positionLabel(p)}</span>
        {#if isGm}
          {#if p.pausedAt == null}
            <button type="button" data-testid="playing-pause" onclick={() => playOp({ type: "pause", id: p.id })}>{t("audio.pause")}</button>
          {:else}
            <button type="button" data-testid="playing-resume" onclick={() => playOp({ type: "resume", id: p.id })}>{t("audio.resume")}</button>
          {/if}
          <button type="button" data-testid="playing-stop" onclick={() => playOp({ type: "stop", id: p.id })}>{t("audio.stop")}</button>
          <button type="button" data-testid="playing-prev" onclick={() => playOp({ type: "prev", id: p.id })}>{t("audio.prev")}</button>
          <button type="button" data-testid="playing-next" onclick={() => playOp({ type: "next", id: p.id })}>{t("audio.next")}</button>
          <input
            type="range"
            min="0"
            max={Math.max(durations.get(p.asset) ?? 0, positionSecs(p) + 60)}
            step="1"
            data-testid="playing-seek"
            aria-label={t("audio.seek")}
            value={positionSecs(p)}
            onchange={(e) =>
              playOp({ type: "seek", id: p.id, position_ms: Math.round(Number((e.currentTarget as HTMLInputElement).value) * 1000) })}
          />
        {/if}
      </li>
    {/each}
  </ul>
  {#if isGm && (audioState?.playing.length ?? 0) > 0}
    <button type="button" data-testid="stop-all" onclick={() => playOp({ type: "stop_all" })}>{t("audio.stopAll")}</button>
  {/if}

  <h3>{t("audio.playlists")}</h3>
  <input type="search" data-testid="playlists-search" aria-label={t("audio.search")} placeholder={t("audio.search")} bind:value={query} />
  <ul class="playlists">
    {#each hits as hit (hit.id)}
      <li data-testid="playlist-row" data-playlist-id={hit.id}>
        <span>{hit.name ?? hit.id}</span>
        {#if isGm}
          <button type="button" data-testid="playlist-play" onclick={() => play(hit.id)}>{t("audio.play")}</button>
        {/if}
        {#if ctx.canDelete(hit)}
          <button type="button" data-testid="playlist-delete" onclick={() => deletePlaylist(hit)}>{t("audio.delete")}</button>
        {/if}
        <button type="button" data-testid="playlist-open" onclick={() => ctx.openDocument({ docId: hit.id })}>{t("audio.open")}</button>
      </li>
    {/each}
  </ul>
  {#if canCreate}
    <input type="text" data-testid="playlists-name" aria-label={t("audio.newPlaylistName")} placeholder={t("audio.newPlaylistName")} bind:value={newName} />
    <button type="button" data-testid="playlists-create" onclick={createPlaylist}>{t("audio.create")}</button>
  {/if}
</section>
