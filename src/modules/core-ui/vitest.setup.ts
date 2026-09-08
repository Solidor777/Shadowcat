// jsdom lacks ResizeObserver and WebGL; stub both so Svelte component init
// completes under tests. Real resize/GL behavior is covered by Playwright.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
// This setup runs in EVERY environment the package selects, and a file declaring the node
// environment has no DOM to patch, so the stub is conditional on the class existing.
if (typeof HTMLCanvasElement !== "undefined") {
  HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
}
