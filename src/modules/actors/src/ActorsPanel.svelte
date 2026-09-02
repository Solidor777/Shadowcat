<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, LightEmissionEditor, VisionAssignmentsEditor, MovementTagsEditor } from "@shadowcat/ui-kit";
  import { buildActorDoc, setNameHidden, actorDisplayName, resolveVisionModes, DEFAULT_LIGHT_EMISSION, type ActorEngine, type LightEmission, type VisionAssignment, type VisionMode, type WireDocument, type FactionRegistryEngine, type Faction, type TokenVisual, type ConditionRegistryEngine, type Condition, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";
  import VisualKindEditor from "./VisualKindEditor.svelte";
  import FaceSwapPalette from "./FaceSwapPalette.svelte";
  import TokenOwnerControl from "./TokenOwnerControl.svelte";
  import TokenRotationControl from "./TokenRotationControl.svelte";
  import TokenLightControl from "./TokenLightControl.svelte";
  import TokenVisionControl from "./TokenVisionControl.svelte";
  import TokenMovementControl from "./TokenMovementControl.svelte";
  import TokenElevationControl from "./TokenElevationControl.svelte";

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
  /** The create form's pending vision-mode assignments (empty = the new actor has no senses).
   * Read via `$state.snapshot` at create time, same anti-Proxy rule as `pendingVisual`. */
  let pendingVision = $state<VisionAssignment[]>([]);
  /** The create form's pending carried light (`null` = the new actor emits nothing). Read via
   * `$state.snapshot` at create time, same anti-Proxy rule as `pendingVisual`. */
  let pendingLight = $state<LightEmission | null>(null);
  /** The create form's pending movement-type tags (empty = the new actor has none). Read via
   * `$state.snapshot` at create time, same anti-Proxy rule as `pendingVisual`. */
  let pendingMovement = $state<string[]>([]);

  // The visual-kind editor is a child component; it reports its current built visual (or null
  // when incomplete) via `onBuild`, and the host consumes it at create time + resets it after.
  let pendingVisual = $state<TokenVisual | null>(null);
  let visualEditor = $state<{
    /** Clears the child editor's own form state back to defaults; called after a successful
     * `create()` via `bind:this`. */
    reset: () => void;
  }>();

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  /** The single selected token's id, if any — drives every per-token control below: the
   * face-swap palette (`FaceSwapPalette`), the ownership override control
   * (`TokenOwnerControl`), the rotation control (`TokenRotationControl`), the carried-light
   * override control (`TokenLightControl`), the vision override control (`TokenVisionControl`),
   * the movement-tag override control (`TokenMovementControl`), and the elevation control
   * (`TokenElevationControl`). */
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

  /** The actor row's raw stored carried-light emission (`engine.light`), or `null`. This RAW
   * read is the OCC pre-image for `commitLight`'s update — never a resolved/defaulted value.
   * @param a The actor document to read.
   * @returns The raw stored emission, or `null` when absent.
   * @example
   * ```
   * // private helper; read by the per-row light toggle + editor
   * declare const a: WireDocument;
   * lightOf(a); // a.engine.light ?? null
   * ```
   */
  const lightOf = (a: WireDocument): LightEmission | null =>
    (a.engine as ActorEngine | undefined)?.light ?? null;

  /** Dispatch a whole-payload `/engine/light` update on actor `a` (the emission is one nested
   * object, so one write carries every field's change). `old` is the raw stored emission.
   * @param a The actor document.
   * @param next The new emission, or `null` to remove the carried light entirely.
   * @example
   * ```
   * // private helper; wired to the per-row light editor's onCommit
   * declare const a: WireDocument;
   * commitLight(a, null);
   * ```
   */
  function commitLight(a: WireDocument, next: LightEmission | null): void {
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/light", old: lightOf(a), new: next }] }]);
  }

  /** The per-row carried-light toggle: on stamps the shared authoring default, off removes the
   * emission.
   * @param a The actor document.
   * @param on Whether the actor should carry a light.
   * @example
   * ```
   * // private helper; wired to the per-row checkbox's onchange
   * declare const a: WireDocument;
   * toggleLight(a, true);
   * ```
   */
  function toggleLight(a: WireDocument, on: boolean): void {
    commitLight(a, on ? { ...DEFAULT_LIGHT_EMISSION } : null);
  }

  /** The resolved vision-mode registry entries the assignment editors' mode selects offer,
   * reactively re-read so a registry edit (game-settings panel) reaches the rows. */
  const visionModes = $derived.by((): VisionMode[] => {
    subscribe();
    return Object.values(resolveVisionModes(ctx.documents));
  });

  /** The actor row's raw stored vision assignments (`engine.vision`), or `null`. This RAW read
   * is the OCC pre-image for `commitVision`'s update — never a resolved/defaulted value.
   * @param a The actor document to read.
   * @returns The raw stored assignment list, or `null` when absent.
   * @example
   * ```
   * // private helper; read by the per-row vision editor
   * declare const a: WireDocument;
   * visionOf(a); // a.engine.vision ?? null
   * ```
   */
  const visionOf = (a: WireDocument): VisionAssignment[] | null =>
    (a.engine as ActorEngine | undefined)?.vision ?? null;

  /** Dispatch a whole-payload `/engine/vision` update on actor `a` (the assignment list is one
   * nested value, so one write carries every row's change). `old` is the raw stored list; an
   * empty list normalizes to `null` (canonical absent — matches the create form).
   * @param a The actor document.
   * @param next The new assignment list.
   * @example
   * ```
   * // private helper; wired to the per-row vision editor's onCommit
   * declare const a: WireDocument;
   * commitVision(a, []);
   * ```
   */
  function commitVision(a: WireDocument, next: VisionAssignment[]): void {
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/vision", old: visionOf(a), new: next.length > 0 ? next : null }] }]);
  }

  /** The actor row's raw stored movement-type tags (`engine.movement`), or `null` when the key
   * is genuinely absent. This RAW read is the OCC pre-image for `commitMovement`'s update —
   * never a resolved/defaulted value.
   * @param a The actor document to read.
   * @returns The raw stored tag list, or `null` when absent.
   * @example
   * ```
   * // private helper; read by the per-row movement editor
   * declare const a: WireDocument;
   * movementOf(a); // a.engine.movement ?? null
   * ```
   */
  const movementOf = (a: WireDocument): string[] | null =>
    (a.engine as ActorEngine | undefined)?.movement ?? null;

  /** Dispatch a whole-payload `/engine/movement` update on actor `a` (the tag list is one
   * nested value, so one write carries every tag's change). `old` is the raw stored list;
   * unlike `vision` there is no null normalization — `ActorEngine.movement` is a required
   * non-null array, so an empty list commits as `[]`.
   * @param a The actor document.
   * @param next The new tag list.
   * @example
   * ```
   * // private helper; wired to the per-row movement editor's onCommit
   * declare const a: WireDocument;
   * commitMovement(a, ["flying"]);
   * ```
   */
  function commitMovement(a: WireDocument, next: string[]): void {
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/engine/movement", old: movementOf(a), new: next }] }]);
  }

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
      vision: pendingVision.length > 0 ? $state.snapshot(pendingVision) : null,
      light: pendingLight ? $state.snapshot(pendingLight) : null,
      movement: $state.snapshot(pendingMovement),
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
    pendingVision = [];
    pendingLight = null;
    pendingMovement = [];
    visualEditor?.reset();
  }
</script>

<section class="actors">
  <h3>{t("actors.title")}</h3>
  <TokenOwnerControl tokenId={selectedTokenId} />
  <TokenRotationControl tokenId={selectedTokenId} />
  <TokenElevationControl tokenId={selectedTokenId} />
  <FaceSwapPalette tokenId={selectedTokenId} />
  <TokenLightControl tokenId={selectedTokenId} />
  <TokenVisionControl tokenId={selectedTokenId} />
  <TokenMovementControl tokenId={selectedTokenId} />
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
          <!-- Per-row vision-assignment list editor; commits whole-payload /engine/vision
               updates with the raw stored list as the OCC pre-image. -->
          <div class="vision-edit" aria-label={t("actors.visionModes")}>
            <span>{t("actors.visionModes")}</span>
            <VisionAssignmentsEditor value={visionOf(a) ?? []} modes={visionModes} onCommit={(next) => commitVision(a, next)} />
          </div>
          <!-- Per-row movement-tag editor; commits whole-payload /engine/movement updates with
               the raw stored list as the OCC pre-image. -->
          <div class="movement-edit" aria-label={t("actors.movementTags")}>
            <span>{t("actors.movementTags")}</span>
            <MovementTagsEditor value={movementOf(a) ?? []} onCommit={(next) => commitMovement(a, next)} />
          </div>
          <!-- Per-row carried-light toggle + editor; commits whole-payload /engine/light updates. -->
          <label>
            <input
              type="checkbox"
              aria-label={t("actors.carriedLight")}
              checked={lightOf(a) !== null}
              onchange={(e) => toggleLight(a, e.currentTarget.checked)}
            />
            {t("actors.carriedLight")}
          </label>
          {#if lightOf(a)}
            <LightEmissionEditor value={lightOf(a)!} onCommit={(next) => commitLight(a, next)} />
          {/if}
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
    <div class="vision-edit">
      <span>{t("actors.visionModes")}</span>
      <VisionAssignmentsEditor value={pendingVision} modes={visionModes} onCommit={(next) => (pendingVision = next)} />
    </div>
    <div class="movement-edit">
      <span>{t("actors.movementTags")}</span>
      <MovementTagsEditor value={pendingMovement} onCommit={(next) => (pendingMovement = next)} />
    </div>
    {#if ctx.role === "gm"}
      <label>
        <input
          type="checkbox"
          aria-label={t("actors.carriedLight")}
          checked={pendingLight !== null}
          onchange={(e) => (pendingLight = e.currentTarget.checked ? { ...DEFAULT_LIGHT_EMISSION } : null)}
        />
        {t("actors.carriedLight")}
      </label>
      {#if pendingLight}
        <LightEmissionEditor value={pendingLight} onCommit={(next) => (pendingLight = next)} />
      {/if}
    {/if}
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
  .vision-edit {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .movement-edit {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  input,
  label,
  button[type="submit"] {
    min-height: 32px;
  }
</style>
