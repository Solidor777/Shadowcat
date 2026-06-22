// jsdom does not implement ResizeObserver, which the Stage host observes the
// canvas container with. A no-op stub lets component init complete under tests;
// real resize behavior is covered by the Playwright suite (real browser).
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}

// jsdom has no WebGL: HTMLCanvasElement.getContext is unimplemented and logs a
// "Not implemented" error whenever a real Pixi backend is mounted in a unit test.
// Return null so Pixi init fails fast (handled by the Stage host's catch) without
// the console noise; real-GL rendering is covered by the Playwright suite.
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
