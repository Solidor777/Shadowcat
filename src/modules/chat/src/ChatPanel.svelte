<script lang="ts">
  import type { Component } from "svelte";
  import { untrack } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildChannelRegistryDoc,
    type ChannelRegistryEngine,
    type WireAudience,
    type WireDocument,
  } from "@shadowcat/core";
  import {
    postTarget,
    RENDER_CAP,
    createChatDerivationCache,
    deriveVisibleDocs,
    computeVisibleWindow,
    type ChatView,
  } from "./channels";
  import { computeUnreadCount, markAllRead, type ChatReadState } from "./unread";
  import { chatUnreadBadge } from "./unreadBadge";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const subscribeContributions = createSubscriber((update) => ctx.contributions.subscribe(update));

  // All chat messages this client has (already redacted per-recipient by the
  // server), parsed + filtered by the active view + sorted + capped to the
  // last RENDER_CAP for render — the store may hold more via search/resync.
  // `derivationCache` carries sort/parse state across reactive re-runs so a
  // mutation to one message only re-derives that message, not the whole
  // history (see `deriveVisibleDocs`); it is reset whenever the
  // active view changes, since membership is view-scoped.
  let view = $state<ChatView>({ kind: "all" });
  let derivationCache = createChatDerivationCache();
  let cachedView: ChatView | undefined;
  const visibleDocs = $derived.by((): WireDocument[] => {
    subscribe();
    if (cachedView !== view) {
      derivationCache = createChatDerivationCache();
      cachedView = view;
    }
    return deriveVisibleDocs(derivationCache, ctx.documents.query("message"), view, RENDER_CAP);
  });

  // Unread tab badge (I3): a per-channel read frontier persisted opaquely via
  // ctx.uiState (chat owns the blob's shape, the shell only stores it). A
  // persisted marker from a prior session is preferred as-is (so genuinely
  // new messages since last visit count as unread even before this mount's
  // first visibility check); with none, the baseline is EVERY message this
  // mount already sees — otherwise a first-ever open would misreport the
  // whole existing history as unread. Read state is over ALL channels the
  // store holds, not just the active `view` — the badge means "unread
  // anywhere in chat", not "unread in the channel currently on screen".
  let readState = $state<ChatReadState>((ctx.uiState.getChatRead() as ChatReadState | null) ?? markAllRead(ctx.documents.query("message")));
  const unreadCount = $derived.by((): number => {
    subscribe();
    return computeUnreadCount(ctx.documents.query("message"), readState, ctx.selfId);
  });
  $effect(() => {
    chatUnreadBadge.set(unreadCount);
  });
  /**
   * Snapshots every message the store currently holds as read, and persists
   * it. Called whenever the panel is confirmed visible — both on a message
   * arriving while already visible and on a hidden→visible reveal (the
   * IntersectionObserver effect below), so a reader actively looking at the
   * tab never accumulates a stale unread count.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // visibility $effect below and the IntersectionObserver reveal handler
   * markRead();
   * ```
   */
  function markRead(): void {
    const next = markAllRead(ctx.documents.query("message"));
    readState = next;
    ctx.uiState.setChatRead(next);
  }
  // This effect never READS `readState` — it only calls `markRead()`, which
  // writes it — so Svelte's dependency tracking (re-runs only on values read
  // during the callback: `subscribe()`, `ctx.documents.query("message")`,
  // `container`, `isVisible(container)`) never re-triggers this same effect
  // from its own write. `unreadCount` DOES read `readState` and recomputes
  // when `markRead()` runs, but `unreadCount` is a separate `$derived`, not a
  // dependency of this effect, so no cycle forms.
  $effect(() => {
    subscribe();
    ctx.documents.query("message");
    if (container && isVisible(container)) markRead();
  });

  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("channel-registry")[0];
  });
  // Malformed-doc fail-safe only (NOT the removal mechanism — see removeChannel):
  // a channel value should never be null in a doc this client wrote, but a
  // directly-edited or legacy doc could still contain one; filter defensively
  // rather than crash on render.
  const channelEntries = $derived.by((): [string, { name: string }][] => {
    const sys = registry?.engine as ChannelRegistryEngine | undefined;
    return Object.entries(sys?.channels ?? {}).filter((e): e is [string, { name: string }] => e[1] != null);
  });

  /**
   * Composer placeholder contract is the CHANNEL's display name ("Message
   * #General"), never the sender's own username — look the post-target
   * channel id up in the registry, falling back to the raw id when
   * unregistered (e.g. "general" before the GM has ever opened the editor,
   * or a legacy channel).
   * @param channelId The post-target channel id (see `postTarget` in `./channels`).
   * @returns The channel's registered display name, or `channelId` itself when unregistered.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // composer slot below
   * channelDisplayName("general");
   * ```
   */
  function channelDisplayName(channelId: string): string {
    return channelEntries.find(([id]) => id === channelId)?.[1].name ?? channelId;
  }

  // GM registry seed (FactionsPanel idiom): reactive subscribe() inside the
  // $effect so a panel mounted before resync populates the store still seeds
  // exactly once, whether the store was empty at mount or fills in later.
  // `seeded` is set true in BOTH branches below — the early-return branch
  // (registry already exists, no dispatch) and the seed branch (before
  // dispatchIntent runs there) — so the seed is once-only per mount even if
  // the dispatch itself fails; a failed seed is not retried within this
  // mount (only on a fresh mount, when `seeded` re-initializes to false).
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    if (ctx.documents.query("channel-registry").length > 0) {
      seeded = true;
      return;
    }
    seeded = true;
    ctx.dispatchIntent([{ op: "create", doc: buildChannelRegistryDoc(ctx.world, { general: { name: "General" } }) }]);
  });

  let editing = $state(false);
  let newChannelName = $state("");
  /**
   * GM channel editor: appends a new channel entry under a fresh random id,
   * named from the trimmed `newChannelName` input (or a placeholder name if
   * left empty), then clears the input.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // editor's "Add" button
   * addChannel();
   * ```
   */
  function addChannel(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const name = newChannelName.trim() || t("chat.channels.newName");
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/channels/${id}`, old: null, new: { name } }] }]);
    newChannelName = "";
  }
  /**
   * GM channel editor: renames one channel entry in place, patching only
   * `name` and preserving the rest of the entry (`cur`) unchanged.
   * @param id The channel's registry key to rename.
   * @param name The new display name.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each
   * // editor row's name input
   * renameChannel("general", "General");
   * ```
   */
  function renameChannel(id: string, name: string): void {
    if (!registry) return;
    const sys = registry.engine as ChannelRegistryEngine;
    const cur = sys.channels[id];
    if (!cur) return;
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/channels/${id}`, old: cur, new: { ...cur, name } }] }]);
  }
  /**
   * GM channel editor: removes a channel entry from the registry, and — if
   * that channel was the active view — falls back to the "All" view so the
   * panel doesn't keep pointing at a channel that no longer exists.
   * @param id The channel's registry key to remove.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each
   * // editor row's remove button
   * removeChannel("general");
   * ```
   */
  function removeChannel(id: string): void {
    if (!registry) return;
    const sys = registry.engine as ChannelRegistryEngine;
    const cur = sys.channels[id];
    if (!cur) return;
    if (view.kind === "channel" && view.id === id) view = { kind: "all" };
    // Whole-field replace (FactionsPanel idiom, see FactionsPanel's
    // own `remove`): `set_pointer` cannot delete an object key (it only ever
    // inserts/replaces a key, never removes one), so this dispatches the
    // full channel map minus the removed key as one update on the parent
    // path, OCC pre-image included. A single-key remove is also available
    // server-side (`FieldChange.remove` + `remove_pointer`) via the client
    // dispatcher `unsetField` — not what this function uses.
    const next = { ...sys.channels };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/engine/channels", old: sys.channels, new: next }] }]);
  }

  // Card + composer instantiation: read the singleton contributions directly
  // (the Surface subscribe/snapshot idiom, NOT <Surface>) so per-instance
  // reactive props (message, showChannel, channel/audience) can be passed.
  const cardComp = $derived.by(() => {
    subscribeContributions();
    return ctx.contributions.contributionsFor("shadowcat.surface:chat.message")[0]?.component as
      | Component<{ message: WireDocument; showChannel: boolean }>
      | undefined;
  });
  const composerComp = $derived.by(() => {
    subscribeContributions();
    return ctx.contributions.contributionsFor("shadowcat.surface:chat.composer")[0]?.component as
      | Component<{ channel: string; audience: WireAudience; placeholderName: string }>
      | undefined;
  });

  let container = $state<HTMLElement | undefined>(undefined);
  let atBottom = $state(true);
  let showNewMessagesPill = $state(false);
  // Non-reactive: this checkpoint's decision is "did the rendered count grow"
  // (a real new message), never "did scroll position change" — a plain closure
  // variable, not $state, so reading it can't itself trigger the effect below.
  let prevMessageCount = 0;
  // A hidden tab (display:none) reads scrollHeight/clientHeight as 0, so a
  // message arriving while hidden must defer the scroll-to-bottom until the
  // panel becomes visible again (see the IntersectionObserver effect below).
  let pendingScrollToBottom = false;

  // Scroll geometry backing the virtualized window below — kept as $state so
  // computeVisibleWindow re-runs reactively, but only written from real
  // measurement points (scroll, scroll-to-bottom, mount, visibility reveal),
  // never inferred.
  let scrollTop = $state(0);
  let clientHeight = $state(0);
  let scrollHeight = $state(0);
  /**
   * Re-measures the messages container's scroll geometry into
   * `scrollTop`/`clientHeight`/`scrollHeight` — the only writer of those
   * three `$state` values (see the comment above them) — so the virtualized
   * window below stays derived from real, current measurements.
   * @example
   * ```
   * // private function; not part of the public API — invoked from
   * // checkAtBottom, scrollToBottom, and the mount/visibility effects below
   * syncScrollState();
   * ```
   */
  function syncScrollState(): void {
    if (!container) return;
    scrollTop = container.scrollTop;
    clientHeight = container.clientHeight;
    scrollHeight = container.scrollHeight;
  }

  // Only the rows within the measured scroll range (plus overscan) are
  // mounted; RENDER_CAP above bounds what's reactively derived at all, this
  // narrows what's actually placed in the DOM within that bound.
  const windowed = $derived.by(() => computeVisibleWindow(scrollTop, clientHeight, scrollHeight, visibleDocs.length));
  const windowedDocs = $derived.by(() => visibleDocs.slice(windowed.start, windowed.end));
  // Circular by construction, not a real average: scrollHeight includes the
  // spacers THIS value sizes (below), so avgRowHeight measures the window's
  // own current layout, not row content. Used only to keep the scrollbar's
  // proportion/position stable as the window moves.
  const avgRowHeight = $derived.by(() => (visibleDocs.length > 0 && scrollHeight > 0 ? scrollHeight / visibleDocs.length : 0));

  /**
   * Recomputes `atBottom` from the container's current scroll geometry (a
   * 4px slack tolerance for sub-pixel/rounding scroll positions) and
   * refreshes scroll state via `syncScrollState`. Bound to the messages
   * container's `onscroll` handler.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // messages container's onscroll handler
   * checkAtBottom();
   * ```
   */
  function checkAtBottom(): void {
    if (!container) return;
    syncScrollState();
    atBottom = container.scrollTop + container.clientHeight >= container.scrollHeight - 4;
  }
  /**
   * Scrolls the messages container to its current maximum `scrollTop`,
   * re-syncs scroll state from that new position, marks `atBottom`, and
   * clears the "new messages" pill and any pending deferred scroll.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the new
   * // messages pill's click handler and the IntersectionObserver reveal
   * scrollToBottom();
   * ```
   */
  function scrollToBottom(): void {
    if (!container) return;
    container.scrollTop = container.scrollHeight;
    syncScrollState();
    atBottom = true;
    showNewMessagesPill = false;
    pendingScrollToBottom = false;
  }
  /**
   * Cheap display:none check: the panel host hides an inactive or
   * compact-mode-inactive panel via `display: none` on an ancestor — the
   * `.staging` container (`PanelHost`'s `stagingEl`, written back into by
   * `releaseToStaging`) a released slot returns to — rather than unmounting
   * it with `{#if}`; checked both `PanelHost` and
   * `CompactSwitcher`, neither ever `{#if}`-removes a mounted panel
   * slot. This forces every descendant's `offsetParent` to `null`, the proxy
   * this function relies on. `offsetParent` is also `null` for a
   * `position: fixed` element; no CSS rule in `src/modules/panels`'s own
   * styles or the vendored `dockview-core` stylesheet sets `position: fixed`
   * (checked both — CSS only). The pinned `dockview-core@7.0.2` DOES apply
   * `position: fixed` via inline JS style in two places — `PointerGhost`'s
   * constructor (a drag ghost) and `TabGroupManager.setGroupDragImage` (a
   * drag-clone wrapper) — but both style a transient drag-ghost/drag-clone
   * element, never the live panel-content element this function's `el` is
   * drawn from; re-check this on any `dockview-core` version bump.
   * @param el The element to test.
   * @returns `true` if `el` is laid out (has an `offsetParent`), `false` if
   * hidden via `display: none` on an ancestor.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // visibility $effect (which gates markRead) and the message-count/
   * // IntersectionObserver effects below
   * isVisible(container);
   * ```
   */
  function isVisible(el: HTMLElement): boolean {
    return el.offsetParent !== null;
  }

  $effect(() => {
    // `visibleDocs.length` is read unconditionally every run, so it is
    // always a dependency. `container` (read here and via
    // isVisible(container) below) is read only when `grew` is true, because
    // of the `!grew || !container` short-circuit below — so it is NOT a
    // dependency on a run where the message count didn't grow, but IS one on
    // a run where it did.
    const count = visibleDocs.length;
    const grew = count > prevMessageCount;
    prevMessageCount = count;
    if (!grew || !container) return;
    // untrack: read atBottom's CURRENT value without subscribing this effect to
    // it — onscroll-driven atBottom writes must never re-run this effect on
    // their own (untrack is what guarantees that). Read once and reuse for
    // both the hidden and visible branches below, so a hidden reader who had
    // scrolled up keeps that position (and gets the pill) on reveal instead of
    // being force-scrolled to the bottom, mirroring the visible path.
    const wasAtBottom = untrack(() => atBottom);
    if (!isVisible(container)) {
      // Do not measure or write scrollTop while hidden — scrollHeight/
      // clientHeight both read 0, which would wrongly zero scrollTop and mark
      // the panel "at bottom." Resync happens on the next visibility transition.
      if (wasAtBottom) {
        pendingScrollToBottom = true;
      } else {
        showNewMessagesPill = true;
      }
      return;
    }
    if (wasAtBottom) {
      // queueMicrotask, not a synchronous call — a microtask runs BEFORE the
      // next paint, not after one. Deferred so scrollToBottom's scrollHeight
      // read happens on the next microtask turn rather than inline here.
      queueMicrotask(scrollToBottom);
    } else {
      showNewMessagesPill = true;
    }
  });

  // Hidden tabs never fire scroll/resize; IntersectionObserver is the mechanism
  // that DOES fire across a display:none <-> visible transition (an element
  // with no layout box while display:none re-enters the observer's intersection
  // set once it's laid out again), so it is the resync signal for a
  // scroll-to-bottom deferred by the effect above while the tab was inactive.
  $effect(() => {
    if (!container) return;
    const el = container;
    const observer = new IntersectionObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (!entry?.isIntersecting) return;
      markRead();
      if (pendingScrollToBottom) scrollToBottom();
      else syncScrollState();
    });
    observer.observe(el);
    return () => observer.disconnect();
  });

  // Establishes real scroll geometry once the container mounts (re-runs
  // harmlessly if the element reference ever changes).
  $effect(() => {
    if (!container) return;
    syncScrollState();
  });
</script>

<section class="chat">
  <div class="strip" role="tablist" aria-label={t("chat.channels")}>
    <button type="button" class:active={view.kind === "all"} onclick={() => (view = { kind: "all" })}>{t("chat.all")}</button>
    {#each channelEntries as [id, c] (id)}
      <button type="button" class:active={view.kind === "channel" && view.id === id} onclick={() => (view = { kind: "channel", id })}>{c.name}</button>
    {/each}
    <button type="button" class:active={view.kind === "gm"} onclick={() => (view = { kind: "gm" })}>{t("chat.gmChannel")}</button>
    {#if ctx.role === "gm"}
      <button type="button" class="edit-toggle" aria-label={t("chat.channels.edit")} aria-pressed={editing} onclick={() => (editing = !editing)}>⚙</button>
    {/if}
  </div>

  {#if editing && ctx.role === "gm"}
    <div class="editor">
      {#each channelEntries as [id, c] (id)}
        <div class="editor-row">
          <input aria-label={t("chat.channels.name")} value={c.name} onchange={(e) => renameChannel(id, e.currentTarget.value)} />
          <button type="button" onclick={() => removeChannel(id)}>{t("chat.channels.remove")}</button>
        </div>
      {/each}
      <div class="editor-row">
        <input aria-label={t("chat.channels.name")} bind:value={newChannelName} placeholder={t("chat.channels.name")} />
        <button type="button" onclick={addChannel}>{t("chat.channels.add")}</button>
      </div>
    </div>
  {/if}

  <div class="messages" bind:this={container} onscroll={checkAtBottom}>
    {#if windowed.start > 0}
      <div class="row-spacer" style="height: {windowed.start * avgRowHeight}px" aria-hidden="true"></div>
    {/if}
    {#each windowedDocs as m (m.id)}
      {#if cardComp}
        {@const Card = cardComp}
        <div data-message-row>
          <Card message={m} showChannel={view.kind === "all"} />
        </div>
      {/if}
    {/each}
    {#if windowed.end < visibleDocs.length}
      <div class="row-spacer" style="height: {(visibleDocs.length - windowed.end) * avgRowHeight}px" aria-hidden="true"></div>
    {/if}
  </div>

  {#if showNewMessagesPill}
    <button type="button" class="new-messages-pill" onclick={scrollToBottom}>{t("chat.newMessages")}</button>
  {/if}

  <div class="composer-slot">
    {#if composerComp}
      {@const Composer = composerComp}
      <Composer {...postTarget(view)} placeholderName={channelDisplayName(postTarget(view).channel)} />
    {/if}
  </div>
</section>

<style lang="scss">
  .chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .strip {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .strip button {
    min-height: 44px;
    min-width: 44px;
  }
  .strip button.active {
    font-weight: 700;
  }
  .edit-toggle {
    margin-left: auto;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .editor-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .editor-row input,
  .editor-row button {
    min-height: 44px;
  }
  .messages {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .new-messages-pill {
    align-self: center;
    min-height: 44px;
  }
  .composer-slot {
    flex: 0 0 auto;
  }
</style>
