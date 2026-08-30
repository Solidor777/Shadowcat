// jsdom lacks ResizeObserver; stub it so Svelte component init completes
// under tests. Real resize behavior is covered by Playwright.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
