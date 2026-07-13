import { test, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";

/** Minimal fake MediaQueryList: matches + change-event dispatch. */
class FakeMediaQueryList {
  matches: boolean;
  #listeners: Array<() => void> = [];
  constructor(matches: boolean) {
    this.matches = matches;
  }
  addEventListener(_type: string, cb: () => void): void {
    this.#listeners.push(cb);
  }
  removeEventListener(_type: string, cb: () => void): void {
    this.#listeners = this.#listeners.filter((l) => l !== cb);
  }
  fire(matches: boolean): void {
    this.matches = matches;
    for (const cb of this.#listeners) cb();
  }
}

// `sizeClass.svelte.ts` reads `matchMedia` once at module load, so the mock must
// be stubbed before the (single, file-scoped) dynamic import below picks it up.
const mql = new FakeMediaQueryList(false); // starts compact
vi.stubGlobal("matchMedia", () => mql);
const { default: Probe } = await import("./__fixtures__/SizeClassProbe.svelte");

test("sizeClass() reflects a mocked matchMedia and updates on listener fire", async () => {
  render(Probe);
  expect(screen.getByTestId("size").textContent).toBe("compact");

  mql.fire(true);
  await waitFor(() => expect(screen.getByTestId("size").textContent).toBe("expanded"));
});
