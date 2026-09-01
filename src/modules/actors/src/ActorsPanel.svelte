<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, setNameHidden, actorDisplayName, type ActorEngine, type WireDocument, type FactionRegistryEngine, type Faction, type TokenVisual, type ConditionRegistryEngine, type Condition, type WireSearchHit, type SubscriptionHandle, type AuraEmission, type SoundEmission, type VfxEmission } from "@shadowcat/core";
  import VisualKindEditor from "./VisualKindEditor.svelte";
  import EmissionEditor from "./EmissionEditor.svelte";
  import FaceSwapPalette from "./FaceSwapPalette.svelte";
  import TokenOwnerControl from "./TokenOwnerControl.svelte";
  import TokenRotationControl from "./TokenRotationControl.svelte";
  import TokenEmissionControl from "./TokenEmissionControl.svelte";

  const ctx = getAppContext();
  const t = ctx.t;

  /** Shape of an actor's `engine.displayName` field, read via a structural cast since the row
   * loop only narrows `a.engine` to `unknown`. */
  type DisplayNameEngineShape = {
    /** The actor's authored display name, distinct from the envelope `name` used for search/
     * ownership; falls back to `name` when unset (`actorDisplayName`). */
    displayName?: string;
  };
  /** Shape of an actor's `engine.faction` field. */
  type FactionEngineShape = {
    /** The assigned faction-registry id, or `null`/absent for none. */
    faction?: string | null;
  };
  /** Shape of an actor's `engine.shape` field. */
  type ShapeEngineShape = {
    /** The token's render shape; falls back to `"square"` when unset. It reaches the resolved
     * footprint on square grids only — `footprint::resolve_footprint_cells` ignores it on hex,
     * where an authored size counts HEXES and a hex tessellation draws no square/circle
     * distinction. */
    shape?: string;
  };
  /** Shape of an actor's `engine.size` field. */
  type SizeEngineShape = {
    /** The actor's grid-cell footprint size; falls back to `{ w: 1, h: 1 }` when unset. */
    size?: {
      /** Width in grid cells. */
      w: number;
      /** Height in grid cells. */
      h: number;
    };
  };
  /** Shape of an actor's `engine.vision` field. */
  type VisionEngineShape = {
    /** The actor's vision-mode assignments; the darkvision row reads/writes the sole
     * `mode: "darkvision"` entry. */
    vision?: {
      /** The vision-modes registry id this assignment applies. */
      mode: string;
      /** The assignment's range in grid cells; `null`/absent inherits the mode's own default. */
      range: number | null;
    }[];
  };

  // Reactive read of the document store (same bridge as Surface): reading
  // `subscribe()` inside the derived registers a dependency so the list re-renders on create.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actorDocs = $derived.by(() => {
    subscribe();
    return ctx.documents.query("actor");
  });

  // Live FTS search. Empty query renders the existing reactive full actor list;
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
  let visualEditor = $state<{
    /** Clears the child editor's own form state back to defaults; called after a successful
     * `create()` via `bind:this`. */
    reset: () => void;
  }>();

  // The emission editor is a controlled child component: this panel OWNS the three pending
  // emission values, fed by its `onAura`/`onSound`/`onVfx` callbacks, and consumes them at
  // create time (snapshotted like `pendingVisual` — see that field's read-site comment).
  let pendingAura = $state<AuraEmission | null>(null);
  let pendingSound = $state<SoundEmission | null>(null);
  let pendingVfx = $state<VfxEmission | null>(null);

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  /** The single selected token's id, if any — drives every per-token control below: the
   * face-swap palette (`FaceSwapPalette`), the ownership override control
   * (`TokenOwnerControl`), and the rotation control (`TokenRotationControl`). */
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

  // Rows come from two sources — a store-resolved document and a search hit
  // (`WireSearchHit.document`, a full `WireDocument` clone) — and `permissions` is non-optional on
  // both. Structural guarantee at each end: server-side, ingress
  // (`validation::validate_property_overrides`) rejects any override pointer
  // `permission::redaction_target` cannot classify, which is everything outside the four content
  // bands (`/name`, `/engine`, `/system`, `/base`), so no stored override can ask egress to null or
  // strip the permissions envelope off a redacted copy; client-side, `SearchHitSchema` parses the
  // hit through `DocumentSchema`, whose `permissions` key is required. No hedge is warranted here.
  const isHidden = (a: WireDocument): boolean => a.permissions.property_overrides["/name"] === "owner_or_gm";

  /**
   * Toggles the `OwnerOrGm` visibility override on `/permissions/property_overrides["/name"]` —
   * the same redaction mechanism `setNameHidden` (`@shadowcat/core`) applies at create time,
   * applied here as a targeted property-override Update instead of a whole-doc rebuild. `old`
   * carries the full pre-image object, matching the server's field-level OCC check.
   * @param a The actor document to toggle name-hiding on.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the per-row "hide name" button
   * declare const actorDoc: WireDocument;
   * toggleHidden(actorDoc);
   * ```
   */
  function toggleHidden(a: WireDocument): void {
    const cur = a.permissions.property_overrides;
    const next = { ...cur };
    if (next["/name"] === "owner_or_gm") delete next["/name"];
    else next["/name"] = "owner_or_gm";
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/permissions/property_overrides", old: cur, new: next }] }]);
  }

  /**
   * Creates a new actor from the form's current fields plus the visual editor's last built
   * `pendingVisual` — a no-op if either the name or the visual is missing, mirroring the submit
   * button's own `disabled` condition. Resets every form field on success, including the visual
   * editor via its exposed `reset()`.
   * @returns Nothing; dispatches a create intent and resets local form state as side effects.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the form's `onsubmit`
   * create();
   * ```
   */
  function create(): void {
    // `$state.snapshot` strips the deep-reactive Proxy wrapping that `pendingVisual` (itself a
    // `$state`) applies to any object/array assigned into it — an unwrapped reactive value
    // embedded in the document would later fail `structuredClone` (the instanced-token
    // deep-copy path in `buildTokenFromActor`), which cannot clone a Proxy. Safe on `null`:
    // Svelte returns non-object values from `$state.snapshot` unchanged.
    const visual = $state.snapshot(pendingVisual);
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
      // Emissions are optional (null = none); snapshotted out of `$state` for the same
      // Proxy/structuredClone reason as `visual` above (emission payloads are flat, but the
      // `$state.snapshot` read-site convention stays uniform).
      aura: $state.snapshot(pendingAura),
      sound: $state.snapshot(pendingSound),
      vfx: $state.snapshot(pendingVfx),
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
    pendingAura = null;
    pendingSound = null;
    pendingVfx = null;
    visualEditor?.reset();
  }
</script>

<section class="actors">
  <h3>{t("actors.title")}</h3>
  <TokenOwnerControl tokenId={selectedTokenId} />
  <TokenRotationControl tokenId={selectedTokenId} />
  <FaceSwapPalette tokenId={selectedTokenId} />
  <TokenEmissionControl tokenId={selectedTokenId} />
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
        >{actorDisplayName({ name: a.name, displayName: (a.engine as DisplayNameEngineShape | undefined)?.displayName })}</button>
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
            value={(a.engine as FactionEngineShape | undefined)?.faction ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/faction", old: (a.engine as FactionEngineShape | undefined)?.faction ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">—</option>
            {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
          </select>
          <select
            aria-label={t("actors.shape")}
            value={(a.engine as ShapeEngineShape | undefined)?.shape ?? "square"}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/shape", old: (a.engine as ShapeEngineShape | undefined)?.shape ?? "square", new: e.currentTarget.value }] }])}
          >
            <option value="square">{t("actors.shapeSquare")}</option>
            <option value="circle">{t("actors.shapeCircle")}</option>
          </select>
          <!-- Per-row size inputs dispatch an update op (not bind:value), so e.currentTarget.value
               is a string; Number(...) coerces it because `ActorEngine.size` is numeric — the
               server reads those fields as `f64` in `footprint::resolve_footprint_cells`. -->
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.width")}
            value={(a.engine as SizeEngineShape | undefined)?.size?.w ?? 1}
            onchange={(e) => { const sz = (a.engine as SizeEngineShape | undefined)?.size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/size", old: sz, new: { w: Number(e.currentTarget.value), h: sz.h } }] }]); }}
          />
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.height")}
            value={(a.engine as SizeEngineShape | undefined)?.size?.h ?? 1}
            onchange={(e) => { const sz = (a.engine as SizeEngineShape | undefined)?.size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/size", old: sz, new: { w: sz.w, h: Number(e.currentTarget.value) } }] }]); }}
          />
          <!-- Per-row darkvision input dispatches an update to /engine/vision; range=0 clears to empty array. -->
          <input
            type="number" min="0" step="1" class="size-edit" aria-label={t("actors.darkvision")}
            value={(a.engine as VisionEngineShape | undefined)?.vision?.find((v) => v.mode === "darkvision")?.range ?? 0}
            onchange={(e) => { const range = Number(e.currentTarget.value); const cur = (a.engine as VisionEngineShape | undefined)?.vision ?? null; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/vision", old: cur, new: range > 0 ? [{ mode: "darkvision", range }] : [] }] }]); }}
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
    <EmissionEditor
      aura={pendingAura}
      sound={pendingSound}
      vfx={pendingVfx}
      onAura={(v) => (pendingAura = v)}
      onSound={(v) => (pendingSound = v)}
      onVfx={(v) => (pendingVfx = v)}
    />
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
