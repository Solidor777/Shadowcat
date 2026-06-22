<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import {
    RenderEngine,
    createPixiBackend,
    type DisplayBackend,
  } from "@shadowcat/render";

  /** Backend factory; defaults to the real Pixi backend. Tests inject a fake
   * (jsdom has no WebGL — real GL is covered by Playwright). */
  let {
    createBackend = (canvas: HTMLCanvasElement): Promise<DisplayBackend> =>
      createPixiBackend(canvas, { background: readColor("--surface-base", 0x101014) }),
  }: {
    createBackend?: (canvas: HTMLCanvasElement) => Promise<DisplayBackend>;
  } = $props();

  const { store, assets, onAssetChanged, subscribeScene } = getAppContext();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;

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
    // Aborts all pointer/wheel listeners on teardown (and on any $effect re-run),
    // so a stale listener set can never call into a destroyed engine.
    const controller = new AbortController();

    void (async () => {
      const backend = await createBackend(canvas);
      if (disposed) { backend.destroy(); return; } // teardown raced the async init
      engine = new RenderEngine({
        store,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--grid-line", 0x363645),
        subscribeScene,
        onDerivedApplied: () => { host.dataset.sceneDerived = "1"; },
      });
      // setViewport (resize + initial grid) then start (camera + reconcile +
      // store subscription). start's applyCamera redraws the grid once more with
      // identical inputs — idempotent initial-frame work, intentional.
      engine.setViewport(host.clientWidth, host.clientHeight);
      engine.start();
      wireCamera(engine, controller.signal);
      // AssetChanged mutates the AssetResolver (cache-bust / placeholder) without a
      // document mutation, so the store-subscription reconcile never fires for it.
      // Re-reconcile explicitly so a replaced/deleted background re-resolves.
      offAsset = onAssetChanged(() => engine?.reconcileNow());
      observer = new ResizeObserver(() => {
        if (engine) engine.setViewport(host.clientWidth, host.clientHeight);
      });
      observer.observe(host);
      host.dataset.renderReady = "true";
    })().catch(() => {
      // Pixi init failed (e.g. no WebGL context). Mark the host so the failure is
      // observable rather than an unhandled rejection; real-GL init is covered by
      // the Playwright suite.
      if (host) host.dataset.renderError = "true";
    });

    return () => {
      disposed = true;
      offAsset?.();
      controller.abort();
      observer?.disconnect();
      engine?.destroy();
    };
  });

  /** Pointer/wheel gestures → camera. Unified pointer events (#10). Listeners are
   * bound to `signal` so teardown removes them all in one `abort()`. */
  function wireCamera(engine: RenderEngine, signal: AbortSignal): void {
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    canvas.addEventListener("pointerdown", (e) => {
      dragging = true; lastX = e.clientX; lastY = e.clientY;
      canvas.setPointerCapture(e.pointerId);
    }, { signal });
    canvas.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      engine.camera.panBy(e.clientX - lastX, e.clientY - lastY);
      lastX = e.clientX; lastY = e.clientY;
      engine.applyCamera();
    }, { signal });
    const endDrag = (): void => { dragging = false; };
    canvas.addEventListener("pointerup", endDrag, { signal });
    canvas.addEventListener("pointercancel", endDrag, { signal });
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
</style>
