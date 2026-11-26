// Shared vitest setup file, listed first in a jsdom package's `setupFiles`.
//
// Node exposes a built-in global `localStorage` that is a non-functional shell (no Storage
// methods) unless the process was started with a storage file, and the jsdom environment's
// global-population skips rebinding keys the Node global already has, so tests would see the
// broken Node shell instead of the jsdom window's real Storage. Rebind it (the jsdom environment
// publishes its window on the `jsdom` global before setup files run). Runs in EVERY environment
// a package selects, so it is a no-op where no jsdom window exists.
const jsdomWindow = (globalThis as { jsdom?: { window: Window } }).jsdom?.window;
if (jsdomWindow && typeof globalThis.localStorage?.setItem !== "function") {
  Object.defineProperty(globalThis, "localStorage", {
    get: () => jsdomWindow.localStorage,
    configurable: true,
  });
}
