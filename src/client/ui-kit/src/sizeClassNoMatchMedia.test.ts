import { test, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";

// jsdom's default test environment provides no `matchMedia`; verify the
// fail-open default before the (single, file-scoped) dynamic import below
// evaluates `sizeClass()`'s module-load-time `matchMedia` guard.
vi.stubGlobal("matchMedia", undefined);
const { default: Probe } = await import("./__fixtures__/SizeClassProbe.svelte");

test("sizeClass() defaults to expanded when matchMedia is unavailable (jsdom default)", () => {
  render(Probe);
  expect(screen.getByTestId("size").textContent).toBe("expanded");
});
