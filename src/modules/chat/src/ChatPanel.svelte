<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildChannelRegistryDoc,
    parseMessageSystem,
    type ChannelRegistrySystem,
    type WireAudience,
    type WireDocument,
  } from "@shadowcat/core";
  import { postTarget, inView, byCreation, RENDER_CAP, type ChatView } from "./channels";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const subscribeContributions = createSubscriber((update) => ctx.contributions.subscribe(update));

  // All chat messages this client has (already redacted per-recipient by the
  // server), parsed + filtered by the active view + sorted + capped to the last
  // RENDER_CAP for render — the store may hold more via search/resync.
  let view = $state<ChatView>({ kind: "all" });
  const visibleDocs = $derived.by((): WireDocument[] => {
    subscribe();
    const inViewDocs = ctx.documents
      .query("message")
      .filter((doc) => {
        const sys = parseMessageSystem(doc);
        return sys !== null && inView(view, sys);
      })
      .sort(byCreation);
    return inViewDocs.slice(Math.max(0, inViewDocs.length - RENDER_CAP));
  });

  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("channel-registry")[0];
  });
  // A removed channel's key is set to null (set_pointer inserts, it cannot
  // delete an object key) rather than dropped from the map — filter tombstones.
  const channelEntries = $derived.by((): [string, { name: string }][] => {
    const sys = registry?.system as ChannelRegistrySystem | undefined;
    return Object.entries(sys?.channels ?? {}).filter((e): e is [string, { name: string }] => e[1] != null);
  });

  // GM registry seed (FactionsPanel idiom): reactive subscribe() inside the
  // $effect so a panel mounted before resync populates the store still seeds
  // exactly once, whether the store was empty at mount or fills in later.
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
  function addChannel(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const name = newChannelName.trim() || "New channel";
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/channels/${id}`, old: null, new: { name } }] }]);
    newChannelName = "";
  }
  function renameChannel(id: string, name: string): void {
    if (!registry) return;
    const sys = registry.system as ChannelRegistrySystem;
    const cur = sys.channels[id];
    if (!cur) return;
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/channels/${id}`, old: cur, new: { ...cur, name } }] }]);
  }
  function removeChannel(id: string): void {
    if (!registry) return;
    const sys = registry.system as ChannelRegistrySystem;
    const cur = sys.channels[id];
    if (!cur) return;
    if (view.kind === "channel" && view.id === id) view = { kind: "all" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/channels/${id}`, old: cur, new: null }] }]);
  }

  // Card + composer instantiation: read the singleton contributions directly
  // (the Surface.svelte subscribe/snapshot idiom, NOT <Surface>) so per-instance
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

  function checkAtBottom(): void {
    if (!container) return;
    atBottom = container.scrollTop + container.clientHeight >= container.scrollHeight - 4;
  }
  function scrollToBottom(): void {
    if (!container) return;
    container.scrollTop = container.scrollHeight;
    atBottom = true;
    showNewMessagesPill = false;
  }

  $effect(() => {
    // Depend on rendered-message count so a new message triggers this effect.
    void visibleDocs.length;
    if (!container) return;
    if (atBottom) {
      // Wait for the DOM to paint the new message before measuring scrollHeight.
      queueMicrotask(scrollToBottom);
    } else {
      showNewMessagesPill = true;
    }
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
    {#each visibleDocs as m (m.id)}
      {#if cardComp}
        {@const Card = cardComp}
        <Card message={m} showChannel={view.kind === "all"} />
      {/if}
    {/each}
  </div>

  {#if showNewMessagesPill}
    <button type="button" class="new-messages-pill" onclick={scrollToBottom}>{t("chat.newMessages")}</button>
  {/if}

  <div class="composer-slot">
    {#if composerComp}
      {@const Composer = composerComp}
      <Composer {...postTarget(view)} placeholderName={ctx.members.get(ctx.selfId) ?? ""} />
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
