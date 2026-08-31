<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, ownerFloorApplies, type WireDocument, type LightEngine, type WallEngine, type FalloffCurve } from "@shadowcat/core";
  import { ToolController, type ToolId, type DrawMode, type TemplateMode, type RegionShapeMode, type RegionBehaviorMode } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";

  const ctx = getAppContext();
  // The controller is fixed per ToolRail instance; capturing the context once is intended.
  // svelte-ignore state_referenced_locally
  const controller = new ToolController({
    scene: ctx.scene,
    actorSelection: ctx.actorSelection,
    tokenSelection: ctx.tokenSelection,
    dispatchIntent: ctx.dispatchIntent,
    documents: ctx.documents,
    assets: ctx.assets,
    world: ctx.world,
    role: ctx.role,
    sendPing: ctx.sendPing,
    pathfind: ctx.pathfind,
    moveRequest: ctx.moveRequest,
    viewedSceneId: () => ctx.viewedSceneId,
    footprints: () => ctx.footprints,
  });
  const t = ctx.t;
  // Authoring is GM-gated (the server is authoritative; this hides the controls).
  // Gating is PER TOOL, not per component: the controller is constructed for every user so
  // a player has an active tool at all — without one, a canvas drag falls through to camera pan.
  const isGm = ctx.role === "gm";

  // Compact: the rail renders as a horizontal bottom strip (core-ui repositions
  // it into the compact grid's bottom row); expanded: a vertical side rail.
  const compact = $derived(sizeClass() === "compact");

  // Reactive subscription mirrors GameSettingsPanel's registry-seed pattern: calling
  // subscribe() inside each $derived.by registers a reactive dependency on the document
  // store so the snap toggle re-resolves as the active scene's doc changes.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const activeScene = $derived.by((): WireDocument | undefined => {
    subscribe();
    const vsid = ctx.viewedSceneId;
    return vsid ? ctx.documents.get(vsid) : ctx.documents.query("scene")[0];
  });
  const snapToGrid = $derived.by((): boolean => {
    subscribe();
    return resolveSceneSettings(activeScene, ctx.documents).snapToGrid;
  });

  /** GM-authored scene-level snap toggle: writes the engine-owned
   * `/engine/snapToGrid` field on the active scene document (shared, not local UI state).
   * No-op with no active scene.
   * @example
   * ```
   * toggleSnap();
   * ```
   */
  function toggleSnap(): void {
    const scene = activeScene;
    if (!scene) return;
    // Reads the RAW stored value (not the resolved/defaulted `snapToGrid`) for optimistic-
    // concurrency `old`: the server's field-level conflict check compares against the actual
    // stored value at this path, which is only `null` while the field is genuinely absent.
    // Mirrors `commitMoves`'s GM branch convention (`eng?.x ?? null`).
    const rawSnap = (scene.engine as {
      /** The raw stored snap flag; absent (not merely falsy) means "use the derived default". */
      snapToGrid?: boolean;
    } | undefined)?.snapToGrid ?? null;
    ctx.dispatchIntent([
      { op: "update", doc_id: scene.id, changes: [{ path: "/engine/snapToGrid", old: rawSnap, new: !snapToGrid }] },
    ]);
  }

  /** `gmOnly` marks a tool that AUTHORS scene content (creates or edits a document other
   * than a token's own position). The three ungated tools each go through a path the server
   * already polices for a non-GM — but select/move does NOT go through the same path for every
   * role: `commitMoves` branches on `ctx.role`. A GM writes
   * `/engine/x,y` directly via `dispatchIntent` — an ordinary permission-checked `Update`, with
   * NO movement gate at all, since `Room::publish`'s non-GM position-refusal block
   * does not apply to a GM's own write. A non-GM's move is
   * request-only and never writes `/engine/x,y` directly: per selected token, `Pathfind` then
   * `MoveRequest` → `execute_move`'s per-cell wall/mask/footprint gate — the same mechanism the
   * measure tool's `commitRoute` uses. Measure previews via the per-requester-masked `Pathfind`
   * and commits a route the same request-only way (`MoveRequest`/`execute_move`); ping is the
   * rate-limited per-user relay.
   *
   * `place` in particular MUST stay `gmOnly`, for a reason that is not fully visible from this
   * file: `Room::publish`'s `Create` gate authorizes a
   * placed token's CENTER cell against the requester's visibility mask (matching
   * `movement_restriction` — no check at all under `Unrestricted`) but NEVER checks walls: a
   * placement is a single point, not a traversal, so there is no `line_traversal`/supercover call
   * to run a wall test against. A `Create` can
   * therefore still place a token behind/through a wall the
   * movement gate would otherwise block. The other server-side check on a player `Create` is
   * `apply_intent`'s `core:create` world-capability grant, which is world-CONFIGURABLE —
   * fail-closed by default, but a world that granted it would have a real wall-bypass placement
   * hole if this tool were ungated. Ungating an authoring tool therefore requires checking what
   * gates its op KIND, not just that some gate exists on the path. */
  const tools: {
    /** The tool id, passed to `controller.toggle`. */
    id: ToolId;
    /** The button's visible label and title (already localized via `t`). */
    label: string;
    /** Whether the tool authors content and must therefore hide from non-GMs (see the doc
     * comment above this array for the per-tool rationale). */
    gmOnly: boolean;
  }[] = [
    { id: "select", label: t("tools.select"), gmOnly: false },
    { id: "place", label: t("tools.place"), gmOnly: true },
    { id: "draw", label: t("tools.draw"), gmOnly: true },
    { id: "template", label: t("tools.template"), gmOnly: true },
    { id: "measure", label: t("tools.measure"), gmOnly: false },
    { id: "ping", label: t("tools.ping"), gmOnly: false },
    { id: "wall", label: t("tools.wall"), gmOnly: true },
    { id: "region", label: t("tools.region"), gmOnly: true },
    { id: "light", label: t("tools.light"), gmOnly: true },
  ];
  const visibleTools = tools.filter((tool) => isGm || !tool.gmOnly);
  // Speak-as-token affordance: shown whenever exactly one token is selected and the current
  // user may plausibly speak as it (GM, or the effective owner) — advisory only, mirroring the
  // "Speak as" composer picker's own client-side offer/server-reauthorizes split. Reuses the
  // subscriber bridge already established above for `activeScene`/`snapToGrid`.
  const selectedSpeakToken = $derived.by((): WireDocument | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size !== 1) return null;
    const [id] = ids;
    return ctx.documents.get(id) ?? null;
  });
  const canSpeakAsSelected = $derived.by((): boolean => {
    subscribe();
    const tok = selectedSpeakToken;
    if (!tok) return false;
    return ctx.role === "gm" || ownerFloorApplies(tok, ctx.selfId, ctx.documents);
  });

  /** Sets the pending speak-as-token selection from the currently selected token (a no-op if
   * none is selected, guarded by `canSpeakAsSelected` at the call site).
   * @example
   * ```
   * // internal; wired to the "speak as this token" button
   * speakAsSelectedToken();
   * ```
   */
  function speakAsSelectedToken(): void {
    const tok = selectedSpeakToken;
    if (tok) ctx.speakAsToken.select(tok.id);
  }

  const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  const templateModes: TemplateMode[] = ["circle", "cone", "rect", "line"];
  const regionShapeModes: RegionShapeMode[] = ["rect", "circle", "polygon"];
  const regionBehaviors: RegionBehaviorMode[] = ["terrain", "impassable", "arrest"];
  const falloffCurves: FalloffCurve[] = ["linear", "quadratic", "none"];

  // The light/wall editor's target document, resolved live from the shared editing selection
  // (reactive: re-reads on every store commit, so the inputs reflect the server's state).
  const editingDoc = $derived.by((): WireDocument | null => {
    subscribe();
    const sel = controller.editingEntity;
    if (!sel) return null;
    const doc = ctx.documents.get(sel.id);
    // A deleted/elsewhere-moved doc drops the selection rather than editing a ghost.
    if (!doc || doc.doc_type !== sel.kind) return null;
    return doc;
  });
  // Keep the controller's selection honest when the target disappears.
  $effect(() => {
    if (controller.editingEntity && !editingDoc) controller.editingEntity = null;
  });

  /** Dispatch one field-level update against the light/wall being edited. `old` MUST be the
   * RAW stored value at `path` (the server's field-level optimistic-concurrency check compares
   * against it) — never a resolved/defaulted value. The standing raw-`old` convention.
   * @param path The engine-band JSON pointer being written.
   * @param oldRaw The raw stored value currently at `path` (`null` when absent).
   * @param value The new value to write.
   * @example
   * ```
   * editSelected("/engine/emission/enabled", true, false);
   * ```
   */
  function editSelected(path: string, oldRaw: unknown, value: unknown): void {
    const doc = editingDoc;
    if (!doc) return;
    ctx.dispatchIntent([{ op: "update", doc_id: doc.id, changes: [{ path, old: oldRaw, new: value }] }]);
  }

  /** Delete the document open in the editor (full pre-image op) and clear the selection.
   * @example
   * ```
   * deleteSelected();
   * ```
   */
  function deleteSelected(): void {
    const doc = editingDoc;
    if (!doc) return;
    ctx.dispatchIntent([{ op: "delete", doc }]);
    controller.editingEntity = null;
  }

  /** Escape backs out of an open light/wall editor (clears the editing selection). Tool
   * deactivation itself stays with the rail's re-select-clears rule (`controller.toggle`).
   * @param event The window keydown event.
   * @example
   * ```
   * onWindowKeydown(new KeyboardEvent("keydown", { key: "Escape" }));
   * ```
   */
  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && controller.editingEntity) controller.editingEntity = null;
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="tool-rail" class:compact role="toolbar" aria-label={t("tools.title")}>
  {#each visibleTools as tool (tool.id)}
    <button
      type="button"
      class="tool"
      class:active={controller.active === tool.id}
      aria-pressed={controller.active === tool.id}
      data-testid="tool-{tool.id}"
      title={tool.label}
      onclick={() => controller.toggle(tool.id)}
    >
      {tool.label}
    </button>
  {/each}

  <!-- Snap is a scene-document write (`/engine/snapToGrid`), i.e. authoring: GM-only. -->
  {#if isGm && activeScene}
    <button
      type="button"
      class="tool"
      aria-pressed={snapToGrid}
      data-testid="snap-toggle"
      title={t("tools.snap")}
      onclick={toggleSnap}
    >
      {t("tools.snap")}
    </button>
  {/if}

  {#if canSpeakAsSelected}
    <button
      type="button"
      class="tool"
      data-testid="speak-as-token"
      title={t("tools.speakAsToken")}
      onclick={speakAsSelectedToken}
    >
      {t("tools.speakAsToken")}
    </button>
  {/if}

  <!-- Every mode control below belongs to a gmOnly tool. Gated on `isGm` as well as the
       active tool so the branch cannot render even if an authoring tool somehow became
       active for a non-GM. -->
  {#if isGm}
    {#if controller.active === "place"}
      <AssetPicker {controller} />
    {:else if controller.active === "draw"}
      <div class="controls">
        <select data-testid="draw-mode" aria-label={t("tools.shape")} bind:value={controller.drawMode}>
          {#each drawModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="draw-color" aria-label={t("tools.color")} bind:value={controller.strokeColor} />
      </div>
    {:else if controller.active === "template"}
      <div class="controls">
        <select data-testid="template-mode" aria-label={t("tools.shape")} bind:value={controller.templateMode}>
          {#each templateModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="template-color" aria-label={t("tools.color")} bind:value={controller.templateColor} />
      </div>
    {:else if controller.active === "region"}
      <div class="controls">
        <select data-testid="region-shape" aria-label={t("tools.shape")} bind:value={controller.regionShapeMode}>
          {#each regionShapeModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <select data-testid="region-behavior" aria-label={t("tools.behavior")} bind:value={controller.regionBehavior}>
          {#each regionBehaviors as b (b)}<option value={b}>{b}</option>{/each}
        </select>
        <input type="number" data-testid="region-cost" aria-label={t("tools.cost")} min="1" step="0.5" bind:value={controller.regionCost} disabled={controller.regionBehavior !== "terrain"} />
        <label>
          <input type="checkbox" data-testid="region-secret" bind:checked={controller.regionSecret} />
          {t("tools.secret")}
        </label>
      </div>
    {/if}

    <!-- The light/wall editors key off the shared editing selection, not the active tool:
         both the select tool and the light tool can open one. `onchange` (commit-on-blur /
         picker-close), never `oninput`, so a slider/color drag doesn't fire one OCC write per
         pixel; each write's `old` reads the RAW stored value from the freshly derived doc. -->
    {#if editingDoc && controller.editingEntity?.kind === "light"}
      {@const eng = editingDoc.engine as LightEngine}
      <div class="controls" data-testid="light-editor">
        <label>
          <input
            type="checkbox"
            data-testid="light-enabled"
            checked={eng.emission.enabled}
            onchange={(e) => editSelected("/engine/emission/enabled", eng.emission.enabled, e.currentTarget.checked)}
          />
          {t("tools.enabled")}
        </label>
        <input
          type="color"
          data-testid="light-color"
          aria-label={t("tools.color")}
          value={/^#[0-9a-fA-F]{6}$/.test(eng.emission.color) ? eng.emission.color : "#ffffff"}
          onchange={(e) => editSelected("/engine/emission/color", eng.emission.color, e.currentTarget.value)}
        />
        <label>
          {t("tools.intensity")}
          <input
            type="number"
            data-testid="light-intensity"
            min="0"
            max="1"
            step="0.05"
            value={eng.emission.intensity}
            onchange={(e) => editSelected("/engine/emission/intensity", eng.emission.intensity, e.currentTarget.valueAsNumber)}
          />
        </label>
        <label>
          {t("tools.brightRadius")}
          <input
            type="number"
            data-testid="light-bright"
            min="0"
            step="0.5"
            value={eng.emission.brightRadius}
            onchange={(e) => editSelected("/engine/emission/brightRadius", eng.emission.brightRadius, e.currentTarget.valueAsNumber)}
          />
        </label>
        <label>
          {t("tools.dimRadius")}
          <input
            type="number"
            data-testid="light-dim"
            min="0"
            step="0.5"
            value={eng.emission.dimRadius}
            onchange={(e) => editSelected("/engine/emission/dimRadius", eng.emission.dimRadius, e.currentTarget.valueAsNumber)}
          />
        </label>
        <select
          data-testid="light-falloff"
          aria-label={t("tools.falloff")}
          value={eng.emission.falloff?.curve ?? "linear"}
          onchange={(e) =>
            editSelected("/engine/emission/falloff", eng.emission.falloff ?? null, { curve: e.currentTarget.value })}
        >
          {#each falloffCurves as f (f)}<option value={f}>{f}</option>{/each}
        </select>
        <button type="button" class="tool" data-testid="light-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {:else if editingDoc && controller.editingEntity?.kind === "wall"}
      {@const eng = editingDoc.engine as WallEngine}
      <div class="controls" data-testid="wall-editor">
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-sight"
            checked={eng.blocksSight}
            onchange={(e) => editSelected("/engine/blocksSight", eng.blocksSight, e.currentTarget.checked)}
          />
          {t("tools.blocksSight")}
        </label>
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-move"
            checked={eng.blocksMove}
            onchange={(e) => editSelected("/engine/blocksMove", eng.blocksMove, e.currentTarget.checked)}
          />
          {t("tools.blocksMove")}
        </label>
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-light"
            checked={eng.blocksLight}
            onchange={(e) => editSelected("/engine/blocksLight", eng.blocksLight, e.currentTarget.checked)}
          />
          {t("tools.blocksLight")}
        </label>
        <button type="button" class="tool" data-testid="wall-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {/if}
  {/if}
</div>

<style lang="scss">
  .tool-rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .tool {
    min-height: 44px; /* touch target (#10) */
    min-width: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .tool.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .controls select,
  .controls input {
    min-height: 32px;

    @media (pointer: coarse) {
      min-height: 44px;
    }
  }

  /* Compact bottom strip: lay tools out horizontally with overflow scroll
   * instead of a vertical column; the active-tool controls follow suit. */
  .tool-rail.compact {
    flex-direction: row;
    flex-wrap: nowrap;
    align-items: center;
    overflow-x: auto;
  }
  .tool-rail.compact .controls {
    flex-direction: row;
    align-items: center;
  }
</style>
