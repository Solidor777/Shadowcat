<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, type WireDocument } from "@shadowcat/core";
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
  // store so the snap toggle re-resolves as the active scene's doc changes (M10f-3 §4.4).
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

  /** GM-authored scene-level snap toggle (M10f-3 §4.4): writes the engine-owned
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
    // Mirrors controller.svelte.ts's commitMoves GM branch convention (`eng?.x ?? null`).
    const rawSnap = (scene.engine as { snapToGrid?: boolean } | undefined)?.snapToGrid ?? null;
    ctx.dispatchIntent([
      { op: "update", doc_id: scene.id, changes: [{ path: "/engine/snapToGrid", old: rawSnap, new: !snapToGrid }] },
    ]);
  }

  /** `gmOnly` marks a tool that AUTHORS scene content (creates or edits a document other
   * than a token's own position). The three ungated tools each go through a path the server
   * already polices for a non-GM — but select/move does NOT go through the same path for every
   * role: `commitMoves` (`controller.svelte.ts`) branches on `ctx.role`. A GM writes
   * `/engine/x,y` directly via `dispatchIntent` — an ordinary permission-checked `Update`, with
   * NO movement gate at all, since `Room::publish`'s non-GM position-refusal block
   * (`src/server/src/ws/room.rs:285-306`) does not apply to a GM's own write. A non-GM's move is
   * request-only and never writes `/engine/x,y` directly: per selected token, `Pathfind` then
   * `MoveRequest` → `execute_move`'s per-cell wall/mask/footprint gate — the same mechanism the
   * measure tool's `commitRoute` uses. Measure previews via the per-requester-masked `Pathfind`
   * and commits a route the same request-only way (`MoveRequest`/`execute_move`); ping is the
   * rate-limited per-user relay.
   *
   * `place` in particular MUST stay `gmOnly`, for a reason that is not fully visible from this
   * file: `Room::publish`'s `Create` gate (`src/server/src/ws/room.rs:307-368`) authorizes a
   * placed token's CENTER cell against the requester's visibility mask (matching
   * `movement_restriction` — no check at all under `Unrestricted`) but NEVER checks walls: a
   * placement is a single point, not a traversal, so there is no `line_traversal`/supercover call
   * to run a wall test against (deliberate — see the comment at that gate, and ARCHITECTURE
   * invariant 6). A `Create` can therefore still place a token behind/through a wall the
   * movement gate would otherwise block. The other server-side check on a player `Create` is
   * `apply_intent`'s `core:create` world-capability grant, which is world-CONFIGURABLE —
   * fail-closed by default, but a world that granted it would have a real wall-bypass placement
   * hole if this tool were ungated. Ungating an authoring tool therefore requires checking what
   * gates its op KIND, not just that some gate exists on the path. */
  const tools: { id: ToolId; label: string; gmOnly: boolean }[] = [
    { id: "select", label: t("tools.select"), gmOnly: false },
    { id: "place", label: t("tools.place"), gmOnly: true },
    { id: "draw", label: t("tools.draw"), gmOnly: true },
    { id: "template", label: t("tools.template"), gmOnly: true },
    { id: "measure", label: t("tools.measure"), gmOnly: false },
    { id: "ping", label: t("tools.ping"), gmOnly: false },
    { id: "wall", label: t("tools.wall"), gmOnly: true },
    { id: "region", label: t("tools.region"), gmOnly: true },
  ];
  const visibleTools = tools.filter((tool) => isGm || !tool.gmOnly);
  const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  const templateModes: TemplateMode[] = ["circle", "cone", "rect", "line"];
  const regionShapeModes: RegionShapeMode[] = ["rect", "circle", "polygon"];
  const regionBehaviors: RegionBehaviorMode[] = ["terrain", "impassable", "arrest"];
</script>

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
