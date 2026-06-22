<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import {
    RenderEngine,
    createPixiBackend,
    type DisplayBackend,
    type Point,
  } from "@shadowcat/render";

  /** Backend factory; defaults to the real Pixi backend. Tests inject a fake
   * (jsdom has no WebGL — real GL is covered by Playwright). */
  let {
    createBackend = (canvas: HTMLCanvasElement): Promise<DisplayBackend> =>
      createPixiBackend(canvas, { background: readColor("--surface-base", 0x101014) }),
  }: {
    createBackend?: (canvas: HTMLCanvasElement) => Promise<DisplayBackend>;
  } = $props();

  const { documents, assets, onAssetChanged, subscribeScene, scene, onPing, role } = getAppContext();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  /** Live engine handle for the GM vision control (set after async init). */
  let engineRef: RenderEngine | null = null;
  /** GM vision mode: "all" (no fog), "fog" (client-only full-fog preview), or "as:<userId>"
   * (M9c-2 see-as-player: re-subscribe vision as that user — server-gated to GMs). */
  let gmView = $state("all");
  /** Candidate see-as targets: distinct token owners the GM sees (best-effort; usernames need a
   * members source — labeled by short id for now). */
  let playerOptions = $state<string[]>([]);

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

  /** Resolve a CSS custom property (which may be a `var()` alias) to a 0xRRGGBB
   * number by reading the computed `color` off a throwaway probe — getPropertyValue
   * returns the unresolved `var(...)` string for aliased custom properties. */
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
        onDerivedApplied: (input) => { host.dataset.sceneDerived = "1"; host.dataset.visionMode = input.mode; },
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
      wirePointer(e, controller.signal);
      // Drive the grid from the active scene's system.grid (M8d §15), updating only on
      // a real change so a token drag does not rebuild the grid each frame; also expose
      // the rendered token count as a test/observability signal (mirrors render-ready).
      let lastGridKey = "";
      const onDocs = (): void => {
        const g = (documents.query("scene")[0]?.system as { grid?: { kind: "square" | "hex"; size: number } } | undefined)?.grid;
        const spec = g ?? { kind: "square" as const, size: 100 };
        const key = `${spec.kind}:${spec.size}`;
        if (key !== lastGridKey) {
          lastGridKey = key;
          e.setGrid(spec);
        }
        host.dataset.tokenCount = String(documents.query("token").length);
        host.dataset.shapeCount = String(documents.query("drawing").length + documents.query("template").length);
        host.dataset.wallCount = String(documents.query("wall").length);
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
    })().catch(() => {
      // Pixi init failed (e.g. no WebGL context). Mark the host so the failure is
      // observable rather than an unhandled rejection; real-GL init is covered by
      // the Playwright suite.
      if (host) host.dataset.renderError = "true";
    });

    return () => {
      disposed = true;
      engineRef = null;
      detachScene?.();
      offGrid?.();
      offPing?.();
      offAsset?.();
      controller.abort();
      observer?.disconnect();
      engine?.destroy();
    };
  });

  /** Pointer/wheel gestures → the engine's tool-aware dispatcher (active tool first,
   * camera pan as the no-tool fallback). Unified pointer events (#10); listeners are
   * bound to `signal` so teardown removes them all in one `abort()`. */
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
        <option value={`as:${owner}`}>See as {owner.slice(0, 8)}</option>
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
    top: var(--space-2, 0.5rem);
    right: var(--space-2, 0.5rem);
    padding: var(--space-1, 0.25rem) var(--space-2, 0.5rem);
    font-size: 0.8125rem;
    color: var(--text-on-surface, #e8e8f0);
    background: var(--surface-raised, #1c1c24);
    border: 1px solid var(--border-subtle, #363645);
    border-radius: var(--radius-sm, 0.25rem);
    cursor: pointer;
    min-height: 2.25rem; /* touch target (#10) */
  }
</style>
