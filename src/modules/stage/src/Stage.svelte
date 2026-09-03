<script lang="ts">
  import { getAppContext, activeTheme } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, resolveTokenVisual, consoleLogger, type Logger, type SceneEngine } from "@shadowcat/core";
  import {
    RenderEngine,
    createPixiBackend,
    type DisplayBackend,
    type Point,
  } from "@shadowcat/render";
  import { untrack } from "svelte";
  import { createSubscriber } from "svelte/reactivity";

  /** Backend factory; defaults to the real Pixi backend. Tests inject a fake
   * (jsdom has no WebGL — real GL is covered by Playwright). */
  let {
    createBackend = (canvas: HTMLCanvasElement): Promise<DisplayBackend> =>
      createPixiBackend(canvas, { background: readColor("--surface-base", 0x101014) }),
    logger,
  }: {
    /** See the doc comment on the destructured default above. */
    createBackend?: (canvas: HTMLCanvasElement) => Promise<DisplayBackend>;
    /** Diagnostic sink for a backend-init failure; no logger seam exists on
     * AppContext (mirrors `PanelHost`'s identical pattern), so this component
     * accepts one as an optional prop and falls back to the production
     * console logger. */
    logger?: Logger;
  } = $props();
  const log = untrack(() => logger ?? consoleLogger());

  // `ctx` is a live object (AppContext.viewedSceneId reads the session's reactive
  // `gmViewedScene` $state) — kept intact rather than destructured so reads through it
  // stay live; the other fields are stable references, safe to destructure.
  const ctx = getAppContext();
  const { documents, assets, onAssetChanged, subscribeScene, scene, onPing, onEmote, onMoveOutcome, role, members } = ctx;

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  /** Live engine handle for the GM vision control and the theme-swap recolor
   * (set after async init; `$state` so the recolor effect below observes it). */
  let engineRef = $state<RenderEngine | null>(null);
  /** GM vision mode: "all" (no fog), "fog" (client-only full-fog preview), or "as:<userId>"
   * (see-as-player: re-subscribe vision as that user — server-gated to GMs). */
  let gmView = $state("all");
  /** Candidate see-as targets: distinct token owners the GM sees (best-effort; usernames need a
   * members source — labeled by short id for now). */
  let playerOptions = $state<string[]>([]);

  /** Applies the current `gmView` selection to the live engine. `"all"` and `"fog"` are
   * client-only — `"fog"` layers a local full-fog preview overlay, no server round-trip —
   * while `"as:<userId>"` re-subscribes the vision channel as that user and is a
   * server-gated operation: `egress_loop`'s `SceneSubscribe` arm rejects an `as_user`
   * from a non-GM connection outright. Called again after an `$effect` re-run (below,
   * `if (gmView !== "all") applyGmView()`) so a non-default view survives teardown/re-init.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the GM-view <select>'s
   * // onchange handler and from the mount $effect
   * applyGmView();
   * ```
   */
  function applyGmView(): void {
    const v = gmView;
    if (v.startsWith("as:")) {
      engineRef?.setFogPreview(false);
      engineRef?.setViewAsUser(v.slice(3));
    } else {
      // "all" or "fog": the GM's own subscription (no see-as); "fog" adds the full-fog preview.
      engineRef?.setViewAsUser(null);
      engineRef?.setFogPreview(v === "fog");
    }
    host.dataset.gmView = v;
  }

  /** Resolves a CSS custom property NAME (a design token — e.g. `--surface-base`,
   * `--grid-line` — not a game/dice token) to a 0xRRGGBB number, by reading the computed
   * `color` off a throwaway probe span rather than calling `getPropertyValue` directly:
   * `getPropertyValue` returns the unresolved `var(...)` string for an ALIASED custom
   * property, and resolving through the computed style is the only way to get the final
   * color — that indirection is the entire reason this function exists. Falls back to
   * `fallback` in two independent cases: no `getComputedStyle` function or no `host`
   * element yet (SSR / not yet mounted); or a computed `color` string that doesn't parse
   * as `rgb()`/`rgba()`.
   * @param token The CSS custom property name to resolve, e.g. `"--grid-line"`.
   * @param fallback The 0xRRGGBB value returned when resolution fails.
   * @returns The resolved 0xRRGGBB color, or `fallback`.
   * @example
   * ```
   * // private function; not part of the public API
   * readColor("--grid-line", 0x363645);
   * ```
   */
  function readColor(token: string, fallback: number): number {
    if (typeof getComputedStyle !== "function" || !host) return fallback;
    const probe = document.createElement("span");
    probe.style.color = `var(${token})`;
    probe.style.display = "none";
    host.appendChild(probe);
    const rgb = getComputedStyle(probe).color; // "rgb(r, g, b)" or ""
    host.removeChild(probe);
    const m = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(rgb);
    if (!m) return fallback;
    return (Number(m[1]) << 16) | (Number(m[2]) << 8) | Number(m[3]);
  }

  $effect(() => {
    let engine: RenderEngine | null = null;
    let disposed = false;
    let observer: ResizeObserver | null = null;
    let offAsset: (() => void) | null = null;
    let offGrid: (() => void) | null = null;
    let offPing: (() => void) | null = null;
    let offEmote: (() => void) | null = null;
    let offMoveOutcome: (() => void) | null = null;
    let offViewed: (() => void) | null = null;
    let detachScene: (() => void) | null = null;
    // Aborts all pointer/wheel listeners on teardown (and on any $effect re-run),
    // so a stale listener set can never call into a destroyed engine.
    const controller = new AbortController();

    void (async () => {
      const backend = await createBackend(canvas);
      if (disposed) { backend.destroy(); return; } // teardown raced the async init
      engine = new RenderEngine({
        store: documents,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--grid-line", 0x363645),
        subscribeScene,
        viewedSceneId: () => ctx.viewedSceneId,
        footprints: () => ctx.footprints,
        selectedTokens: () => ctx.tokenSelection.ids,
        onDerivedApplied: (input) => {
          host.dataset.sceneDerived = "1";
          host.dataset.visionMode = input.mode;
          // Read-only observability signal: the applied frame's creature-sense token ids,
          // id-sorted so the string is order-independent. This is the set `TokenView` raises
          // above the fog mask — empty under `mode: "all"` and whenever nothing is perceived.
          host.dataset.perceivedTokens = [...input.perceived].sort().join(";");
        },
        onLightingApplied: (frame, sweeping) => {
          // Read-only observability signals: the painted lighting overlay's cell count and
          // whether a carried-light sweep is driving it — an e2e can see a torch light a
          // corridor mid-walk (the count rises while `data-light-sweep` is "1") without
          // reading WebGL pixels. Each attribute is written only when its value changes:
          // the engine paints on every fade tick and sweep step, and a dataset write is a
          // DOM attribute mutation each time.
          const litCells = String(frame.cells.length);
          const lightSweep = sweeping ? "1" : "0";
          // Axial bounding box of the lit cells ("minI,minJ,maxI,maxJ"; "" when nothing is lit)
          // — how far along a corridor the glow currently reaches.
          let minI = Infinity, minJ = Infinity, maxI = -Infinity, maxJ = -Infinity;
          for (const c of frame.cells) {
            if (c.i < minI) minI = c.i;
            if (c.j < minJ) minJ = c.j;
            if (c.i > maxI) maxI = c.i;
            if (c.j > maxJ) maxJ = c.j;
          }
          const litBbox = frame.cells.length === 0 ? "" : `${minI},${minJ},${maxI},${maxJ}`;
          if (host.dataset.litCells !== litCells) host.dataset.litCells = litCells;
          if (host.dataset.lightSweep !== lightSweep) host.dataset.lightSweep = lightSweep;
          if (host.dataset.litBbox !== litBbox) host.dataset.litBbox = litBbox;
        },
      });
      const e = engine;
      // setViewport (resize + initial grid) then start (camera + reconcile +
      // store subscription). start's applyCamera redraws the grid once more with
      // identical inputs — idempotent initial-frame work, intentional.
      e.setViewport(host.clientWidth, host.clientHeight);
      e.start();
      // Tools reach this engine via the AppContext scene bridge.
      detachScene = scene.attach(e);
      engineRef = e;
      if (gmView !== "all") applyGmView(); // survive an $effect re-run with a non-default view
      // Re-project on a client-local viewed-scene switch (activeScene flip or GM roam). Neither
      // carries a new server frame, so the engine must re-filter its views + last vision payload.
      let lastViewed = ctx.viewedSceneId;
      // A "footprints" frame likewise carries no store commit, so the token views need an
      // explicit re-projection when the server states new extents.
      let lastFootprints = ctx.footprints;
      // Token selection is client-local UI state (no document write either) — same explicit
      // re-projection so the selection highlight fx tracks the click.
      let lastSelectionKey = [...ctx.tokenSelection.ids].sort().join(" ");
      const vsSub = createSubscriber((update) => documents.subscribe(update));
      offViewed = $effect.root(() => {
        $effect(() => {
          vsSub(); // track store changes (activeScene doc edits)
          const now = ctx.viewedSceneId; // tracks gmViewedScene $state
          if (now !== lastViewed) {
            lastViewed = now;
            e.reapplyViewedScene();
          }
          const fp = ctx.footprints; // tracks the session's footprints $state
          if (fp !== lastFootprints) {
            lastFootprints = fp;
            e.reapplyFootprints();
          }
          // Iterating the SvelteSet tracks it; a membership change re-projects the tokens.
          const selKey = [...ctx.tokenSelection.ids].sort().join(" ");
          if (selKey !== lastSelectionKey) {
            lastSelectionKey = selKey;
            e.reapplyTokenSelection();
          }
        });
      });
      wirePointer(e, controller.signal);
      // Drive the grid from the viewed scene's `engine.grid`, updating only on
      // a real change so a token drag does not rebuild the grid each frame; also expose
      // the rendered token count as a test/observability signal (mirrors render-ready).
      let lastGridKey = "";
      let lastAnimKey = "";
      const onDocs = (): void => {
        const vsid = ctx.viewedSceneId;
        const activeSceneDoc = vsid ? documents.get(vsid) : documents.query("scene")[0];
        // Resolved once so both diagonalRule and animation read from the same snapshot.
        const settings = resolveSceneSettings(activeSceneDoc, documents);
        const g = (activeSceneDoc?.engine as {
          /** Mirrors the server's `data::engine::scene::Grid`; absent on a scene doc that
           * predates a grid write, falling back to the square/100 default below. */
          grid?: {
            /** `"square"` or `"hex"` — kept a string in v1, mirroring `Grid.kind`. */
            kind: "square" | "hex";
            /** Cell size in scene units; for hex grids the OUTER radius
             * (center-to-vertex circumradius), mirroring `Grid.size`. */
            size: number;
          };
        } | undefined)?.grid;
        // Diagonal rule is world-scoped (world-settings.pathfinding.diagonalRule); resolved
        // here so the ruler reflects the GM's active rule choice without requiring a page reload.
        const diagonalRule = settings.diagonalRule;
        const spec = { ...(g ?? { kind: "square" as const, size: 100 }), diagonalRule };
        const key = `${spec.kind}:${spec.size}:${diagonalRule}`;
        if (key !== lastGridKey) {
          lastGridKey = key;
          e.setGrid(spec);
        }
        // Snap-to-grid is per-scene. Pushed unconditionally each pass — a
        // cheap flag assignment (unlike setGrid's Grid rebuild or setAnimation's config
        // object), so no change-detection gate is needed here.
        e.setSnapEnabled(settings.snapToGrid);
        // Animation config is world-scoped (world-settings.animation); only push to the
        // engine on change so a token drag does not re-push config each frame.
        const anim = settings.animation;
        const animKey = `${anim.speedCellsPerSec}:${anim.easing}`;
        if (animKey !== lastAnimKey) {
          lastAnimKey = animKey;
          e.setAnimation({ speedCellsPerSec: anim.speedCellsPerSec, easing: anim.easing });
        }
        const sceneTokens = documents.query("token").filter((t) => !vsid || t.parent_id === vsid);
        host.dataset.tokenCount = String(sceneTokens.length);
        // Read-only observability signal: each viewed-scene token's COMMITTED
        // `/engine/x,y` as `id:x,y`, id-sorted so the string is order-independent of
        // the store's iteration. Mirrors data-token-count/data-last-ping. Because the
        // canvas renders the optimistic view, a server-rejected move reverts this
        // string to its pre-drag value — the only DOM-visible signal of a rollback,
        // which a position-less count cannot express.
        host.dataset.tokenPositions = sceneTokens
          .map((t) => {
            const e = t.engine as {
              /** Committed center X in scene units, mirroring `TokenEngine.x`; absent on
               * a token doc that predates placement. */
              x?: number;
              /** Committed center Y in scene units, mirroring `TokenEngine.y`. */
              y?: number;
            } | undefined;
            return `${t.id}:${e?.x ?? 0},${e?.y ?? 0}`;
          })
          .sort()
          .join(";");
        // Read-only observability signal: each viewed-scene token's RESOLVED visual kind
        // (`resolveTokenVisual` — the same read the render layer draws from) as `id:kind`,
        // id-sorted like data-token-positions; `none` when the visual fails closed (the token
        // then also doesn't draw). Lets an assertion confirm an authored visual shape reaches
        // the render boundary without inspecting WebGL pixels directly.
        host.dataset.tokenVisuals = sceneTokens
          .map((t) => `${t.id}:${resolveTokenVisual(t, documents)?.kind ?? "none"}`)
          .sort()
          .join(";");
        host.dataset.shapeCount = String(
          documents.query("drawing").length + documents.query("template").length,
        );
        host.dataset.wallCount = String(documents.query("wall").length);
        // Read-only observability signal: each viewed-scene token's last-projected badge chips
        // (condition glyphs, then the elevation chip) as `id:chip,chip`, id-sorted — the same
        // string list `PixiBackend` turns into the canvas's upright Text nodes, so an e2e can
        // confirm a badge reached the render layer without inspecting WebGL pixels. Reads AFTER
        // the engine's own store subscription (registered in `start`, before this one) has
        // reconciled the specs this commit.
        host.dataset.tokenBadges = sceneTokens
          .map((t) => `${t.id}:${(e.badgesForTest(t.id) ?? []).join(",")}`)
          .sort()
          .join(";");
        // Read-only observability signal mirroring the reconciler's own background
        // resolution (the viewed scene's `engine.background`) — "" when unset, so an
        // e2e assertion can confirm the authored background reached the render layer
        // without inspecting WebGL pixels directly.
        host.dataset.background = (activeSceneDoc?.engine as SceneEngine | undefined)?.background ?? "";
        // See-as-player candidates: distinct token owners the GM sees (best-effort labels).
        playerOptions = [...new Set(documents.query("token").map((t) => t.owner).filter((o): o is string => !!o))];
        // If the selected see-as target's token left, fall back to "See all" (drops the stale sub).
        if (gmView.startsWith("as:") && !playerOptions.includes(gmView.slice(3))) {
          gmView = "all";
          applyGmView();
        }
      };
      onDocs();
      offGrid = documents.subscribe(onDocs);
      // Relayed pings (incl. our own echo) spawn a transient ring at scene coords.
      offPing = onPing((m) => {
        e.addPing(m.x, m.y);
        host.dataset.lastPing = `${m.x},${m.y}`;
      });
      // Relayed emotes (incl. our own echo) spawn a transient glyph over the token.
      offEmote = onEmote((m) => {
        e.addEmote(m.token, m.emote);
        host.dataset.lastEmote = `${m.token}:${m.emote}`;
      });
      // Read-only observability signal for the local player's own move requests —
      // no behavior change to movement, just an outcome the client already
      // receives via `WorldSession.moveRequest`'s resolution.
      offMoveOutcome = onMoveOutcome((m) => {
        host.dataset.lastMoveOutcome = m.outcome;
      });
      // AssetChanged mutates the AssetResolver (cache-bust / placeholder) without a
      // document mutation, so the store-subscription reconcile never fires for it.
      // Re-reconcile explicitly so a replaced/deleted background re-resolves.
      offAsset = onAssetChanged(() => e.reconcileNow());
      observer = new ResizeObserver(() => {
        e.setViewport(host.clientWidth, host.clientHeight);
      });
      observer.observe(host);
      host.dataset.gmView = gmView;
      host.dataset.renderReady = "true";
    })().catch((e) => {
      // Pixi init failed (e.g. no WebGL context). Log through the project logger so
      // this is distinguishable from a timeout in e2e output/bug reports, and mark
      // the host so the failure is also observable in the DOM; real-GL init is
      // covered by the Playwright suite.
      log.error("Stage backend init failed", e);
      if (host) host.dataset.renderError = "true";
    });

    return () => {
      disposed = true;
      engineRef = null;
      detachScene?.();
      offGrid?.();
      offPing?.();
      offEmote?.();
      offMoveOutcome?.();
      offAsset?.();
      offViewed?.();
      controller.abort();
      observer?.disconnect();
      engine?.destroy();
    };
  });

  /** Theme-swap recolor: `activeTheme()` is the ui-kit theme controller's
   * `createSubscriber`-backed reactive read, so this effect re-runs on any theme
   * change and pushes the re-read canvas colors into the live engine via
   * `RenderEngine.setThemeColors`. The construction-time reads (the backend
   * factory's `--surface-base`, the `gridColor` opt) remain the initial values;
   * this effect only handles post-construction swaps. */
  $effect(() => {
    activeTheme();
    const e = engineRef;
    if (!e) return;
    e.setThemeColors({
      background: readColor("--surface-base", 0x101014),
      gridColor: readColor("--grid-line", 0x363645),
    });
  });

  /** Pointer/wheel gestures → the engine's tool-aware dispatcher (active tool first,
   * camera pan as the no-tool fallback). Unified pointer events (#10); listeners are
   * bound to `signal` so teardown removes them all in one `abort()`.
   * @param engine The live render engine to dispatch pointer/wheel gestures into.
   * @param signal Aborted on `$effect` teardown/re-run; removes every listener this
   * function registers in one `abort()` call.
   * @example
   * ```
   * declare const engine: RenderEngine;
   * declare const controller: AbortController;
   * // private function; not part of the public API — invoked once per mount $effect run
   * wirePointer(engine, controller.signal);
   * ```
   */
  function wirePointer(engine: RenderEngine, signal: AbortSignal): void {
    const local = (e: PointerEvent): Point => {
      const r = canvas.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    canvas.addEventListener("pointerdown", (e) => {
      canvas.setPointerCapture(e.pointerId);
      engine.dispatchPointerDown(local(e), e);
    }, { signal });
    canvas.addEventListener("pointermove", (e) => engine.dispatchPointerMove(local(e), e), { signal });
    const up = (e: PointerEvent): void => engine.dispatchPointerUp(local(e), e);
    canvas.addEventListener("pointerup", up, { signal });
    canvas.addEventListener("pointercancel", up, { signal });
    canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      engine.camera.zoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
      engine.applyCamera();
    }, { passive: false, signal });
  }
</script>

<div class="stage-host" bind:this={host}>
  <canvas bind:this={canvas} data-testid="stage-canvas"></canvas>
  {#if role === "gm"}
    <select
      class="gm-view"
      data-testid="gm-view-select"
      aria-label="GM vision mode"
      bind:value={gmView}
      onchange={applyGmView}
    >
      <option value="all">See all</option>
      <option value="fog">Preview fog</option>
      {#each playerOptions as owner (owner)}
        <option value={`as:${owner}`}>See as {members.get(owner) ?? owner.slice(0, 8)}</option>
      {/each}
    </select>
  {/if}
</div>

<style lang="scss">
  .stage-host {
    height: 100%;
    width: 100%;
    overflow: hidden;
    background: var(--surface-base);
    touch-action: none; /* let pointer gestures drive pan/zoom on touch (#10) */
  }
  canvas {
    display: block;
  }
  .gm-view {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: var(--space-1) var(--space-2);
    font-size: 0.8125rem;
    color: var(--text-primary);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    cursor: pointer;
    min-height: 2.25rem; /* touch target (#10) */
  }
</style>
