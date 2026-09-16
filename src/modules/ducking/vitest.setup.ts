if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}

// Node exposes a built-in global `localStorage` that is a non-functional shell
// (no Storage methods) unless the process was started with a storage file, and
// the jsdom environment's global-population skips rebinding keys the Node
// global already has — so tests would see the broken Node shell instead of the
// jsdom window's real Storage. Rebind it (the jsdom env publishes its window on
// the `jsdom` global before setup files run).
const jsdomWindow = (globalThis as { jsdom?: { window: Window } }).jsdom?.window;
if (jsdomWindow && typeof globalThis.localStorage?.setItem !== "function") {
  Object.defineProperty(globalThis, "localStorage", {
    get: () => jsdomWindow.localStorage,
    configurable: true,
  });
}
