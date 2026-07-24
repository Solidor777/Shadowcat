<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, setNameHidden, actorDisplayName, type ActorEngine, type WireDocument, type FactionRegistryEngine, type Faction, type TokenVisual, type ConditionRegistryEngine, type Condition, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";
  import VisualKindEditor from "./VisualKindEditor.svelte";
  import FaceSwapPalette from "./FaceSwapPalette.svelte";
  import TokenOwnerControl from "./TokenOwnerControl.svelte";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the document store (same bridge as Surface.svelte): reading
  // `subscribe()` inside the derived registers a dependency so the list re-renders on create.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actorDocs = $derived.by(() => {
    subscribe();
    return ctx.documents.query("actor");
  });

  // Live FTS search (M6c seam). Empty query renders the existing reactive full actor list;
  // a non-empty query drives a top-N subscription keyed on the query string, torn down/recreated
  // on every query change and on unmount (D-c: search is NOT reconnect-resilient, unlike scene
  // subscriptions — a reconnect mid-search leaves the last-known hits until the next keystroke).
  let query = $state("");
  let searchHits = $state<WireDocument[]>([]);
  $effect(() => {
    const q = query.trim();
    if (!q) { searchHits = []; return; }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20 }, (hits: WireSearchHit[]) => {
        // INVARIANT: subscribeSearch's initial page resolves `onUpdate` SYNCHRONOUSLY, inside the
        // pending-resolve handler, BEFORE `resolve({unsubscribe})` runs — so it fires before the
        // `.then()` below (and thus before `cancelled`/`handle` teardown) ever executes. A stale
        // query's callback can therefore still fire after this effect has re-run for a newer query
        // and its own subscription is already active; guard `cancelled` here, not just in `.then()`.
        if (cancelled) return;
        searchHits = hits.filter((h) => h.document.doc_type === "actor").map((h) => h.document);
      })
      .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; handle?.unsubscribe(); };
  });
  const visibleActors = $derived(query.trim() ? searchHits : actorDocs);

  let name = $state("");
  let displayName = $state("");
  let instanceOnDrop = $state(true);
  let hideName = $state(false);
  let faction = $state<string | null>(null);
  let shape = $state<"square" | "circle">("square");
  let sizeW = $state(1);
  let sizeH = $state(1);
  let darkvision = $state(0);

  // The visual-kind editor is a child component; it reports its current built visual (or null
  // when incomplete) via `onBuild`, and the host consumes it at create time + resets it after.
  let pendingVisual = $state<TokenVisual | null>(null);
  let visualEditor = $state<{ reset: () => void }>();

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  /** The single selected token's id, if any — drives the per-token face-swap palette below. */
  const selectedTokenId = $derived.by((): string | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size === 0) return null;
    return ctx.documents.query("token").find((t) => ids.has(t.id))?.id ?? null;
  });

  const factionOptions = $derived.by((): [string, Faction][] => {
    subscribe();
    const reg = ctx.documents.query("faction-registry")[0]?.engine as FactionRegistryEngine | undefined;
    return Object.entries(reg?.factions ?? {});
  });

  // Optional chaining: a live-search hit's document (WireSearchHit.document) carries id/doc_type/
  // system only, not the full permissions envelope — the GM per-row controls below must tolerate
  // a search-sourced row, not just a store-resolved one.
  const isHidden = (a: WireDocument): boolean => a.permissions?.property_overrides["/name"] === "owner_or_gm";

  function toggleHidden(a: WireDocument): void {
    const cur = a.permissions.property_overrides;
    const next = { ...cur };
    if (next["/name"] === "owner_or_gm") delete next["/name"];
    else next["/name"] = "owner_or_gm";
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/permissions/property_overrides", old: cur, new: next }] }]);
  }

  function create(): void {
    const visual = pendingVisual;
    if (!name || !visual) return;
    const engine: ActorEngine = {
      displayName: displayName || name,
      visual,
      size: { w: sizeW, h: sizeH },
      shape,
      faction,
      conditions: [],
      prototype: instanceOnDrop,
      vision: darkvision > 0 ? [{ mode: "darkvision" as const, range: darkvision }] : null,
    };
    const doc = buildActorDoc(ctx.world, name, engine);
    if (hideName) setNameHidden(doc, true);
    ctx.dispatchIntent([{ op: "create", doc }]);
    name = "";
    displayName = "";
    hideName = false;
    faction = null;
    shape = "square";
    sizeW = 1;
    sizeH = 1;
    darkvision = 0;
    visualEditor?.reset();
  }
</script>

<section class="actors">
  <h3>{t("actors.title")}</h3>
  <TokenOwnerControl tokenId={selectedTokenId} />
  <FaceSwapPalette tokenId={selectedTokenId} />
  <input
    class="actor-search"
    type="search"
    placeholder={t("actors.search")}
    aria-label={t("actors.search")}
    bind:value={query}
  />
  <ul class="list">
    {#each visibleActors as a (a.id)}
      <li>
        <button
          type="button"
          class:selected={ctx.actorSelection.selectedId === a.id}
          onclick={() => ctx.actorSelection.select(a.id)}
        >{actorDisplayName({ name: a.name, displayName: (a.engine as { displayName?: string } | undefined)?.displayName })}</button>
        <button type="button" class="open-sheet" onclick={() => ctx.openDocument({ docId: a.id })}>
          {t("actors.openSheet")}
        </button>
        {#if ctx.role === "gm"}
          <button type="button" class="hide-toggle" onclick={() => toggleHidden(a)}>
            {isHidden(a) ? t("actors.nameShown") : t("actors.hideName")}
          </button>
          <!-- Ownership is assigned ONCE here, on the character: every LINKED token
               resolves through it server-side (`effective_owner`), so re-assigning
               re-owns all of them with no per-token write. `old` is the raw stored
               `owner` — the server's field-level OCC check compares against it. -->
          <select
            aria-label={t("actors.actorOwner")}
            value={a.owner ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/owner", old: a.owner ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">{t("actors.ownerNobody")}</option>
            {#each [...ctx.members.entries()] as [uid, uname] (uid)}
              <option value={uid}>{uname}</option>
            {/each}
          </select>
          <select
            aria-label={t("actors.faction")}
            value={(a.engine as { faction?: string | null } | undefined)?.faction ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/faction", old: (a.engine as { faction?: string | null } | undefined)?.faction ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">—</option>
            {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
          </select>
          <select
            aria-label={t("actors.shape")}
            value={(a.engine as { shape?: string } | undefined)?.shape ?? "square"}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/shape", old: (a.engine as { shape?: string } | undefined)?.shape ?? "square", new: e.currentTarget.value }] }])}
          >
            <option value="square">{t("actors.shapeSquare")}</option>
            <option value="circle">{t("actors.shapeCircle")}</option>
          </select>
          <!-- Per-row size inputs dispatch an update op (not bind:value), so e.currentTarget.value
               is a string; Number(...) coerces it to keep engine.size numeric for actor.size × cell math. -->
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.width")}
            value={(a.engine as { size?: { w: number } } | undefined)?.size?.w ?? 1}
            onchange={(e) => { const sz = (a.engine as { size?: { w: number; h: number } } | undefined)?.size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/size", old: sz, new: { w: Number(e.currentTarget.value), h: sz.h } }] }]); }}
          />
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.height")}
            value={(a.engine as { size?: { h: number } } | undefined)?.size?.h ?? 1}
            onchange={(e) => { const sz = (a.engine as { size?: { w: number; h: number } } | undefined)?.size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/size", old: sz, new: { w: sz.w, h: Number(e.currentTarget.value) } }] }]); }}
          />
          <!-- Per-row darkvision input dispatches an update to /engine/vision; range=0 clears to empty array. -->
          <input
            type="number" min="0" step="1" class="size-edit" aria-label={t("actors.darkvision")}
            value={(a.engine as { vision?: Array<{ mode: string; range: number }> } | undefined)?.vision?.find((v) => v.mode === "darkvision")?.range ?? 0}
            onchange={(e) => { const range = Number(e.currentTarget.value); const cur = (a.engine as { vision?: { mode: string; range: number }[] } | undefined)?.vision ?? null; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/vision", old: cur, new: range > 0 ? [{ mode: "darkvision", range }] : [] }] }]); }}
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
    <VisualKindEditor bind:this={visualEditor} conditionOptions={conditionOptions} onBuild={(v) => (pendingVisual = v)} />
    <button type="submit" disabled={!name || !pendingVisual}>{t("actors.create")}</button>
  </form>
</section>

<style lang="scss">
  .actors {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .actor-search {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
  }
  .actor-search:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .open-sheet {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .open-sheet:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
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
  input,
  label,
  button[type="submit"] {
    min-height: 32px;
  }
</style>
