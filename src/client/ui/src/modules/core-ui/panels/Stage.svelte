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

  const { store, assets } = getAppContext();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;

  /** Sample a CSS custom property as a 0xRRGGBB number (canvas chrome reads tokens). */
  function readColor(token: string, fallback: number): number {
    if (typeof getComputedStyle !== "function" || !host) return fallback;
    const raw = getComputedStyle(host).getPropertyValue(token).trim();
    const m = /^#([0-9a-f]{6})$/i.exec(raw);
    return m ? parseInt(m[1], 16) : fallback;
  }

  $effect(() => {
    let engine: RenderEngine | null = null;
    let disposed = false;
    let observer: ResizeObserver | null = null;

    void (async () => {
      const backend = await createBackend(canvas);
      if (disposed) { backend.destroy(); return; } // teardown raced the async init
      engine = new RenderEngine({
        store,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--border", 0x3a3a4a),
      });
      engine.setViewport(host.clientWidth, host.clientHeight);
      engine.start();
      wireCamera(engine);
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
      observer?.disconnect();
      engine?.destroy();
    };
  });

  /** Pointer/wheel gestures → camera. Unified pointer events (#10). */
  function wireCamera(engine: RenderEngine): void {
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    canvas.addEventListener("pointerdown", (e) => {
      dragging = true; lastX = e.clientX; lastY = e.clientY;
      canvas.setPointerCapture(e.pointerId);
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      engine.camera.panBy(e.clientX - lastX, e.clientY - lastY);
      lastX = e.clientX; lastY = e.clientY;
      engine.applyCamera();
    });
    const endDrag = (): void => { dragging = false; };
    canvas.addEventListener("pointerup", endDrag);
    canvas.addEventListener("pointercancel", endDrag);
    canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      engine.camera.zoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
      engine.applyCamera();
    }, { passive: false });
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
