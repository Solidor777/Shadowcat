// jsdom lacks ResizeObserver, IntersectionObserver, and WebGL; stub all three so Svelte
// component init completes under tests. Real resize/intersection/GL behavior is covered by
// Playwright. A test exercising a hidden-tab intersection transition (ChatPanel) overrides this
// default no-op with a controllable stub via vi.stubGlobal.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
if (typeof globalThis.IntersectionObserver === "undefined") {
  globalThis.IntersectionObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof IntersectionObserver;
}
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
