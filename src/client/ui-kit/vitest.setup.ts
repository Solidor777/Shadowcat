// jsdom implements neither ResizeObserver nor WebGL; stub both so Svelte
// component init completes under tests. Real resize/GL behavior is covered by the
// Playwright suite (real browser).
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
