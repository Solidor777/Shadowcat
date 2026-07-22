import { test, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { i18n } from "@shadowcat/ui-kit";
import DockChips from "./DockChips.svelte";

afterEach(() => cleanup());

// The default test `t` (`(k) => k`, no interpolation) can't prove the
// fallback is a real translated string rather than the raw id — use the
// real catalog-backed `i18n.t`, mirroring PanelHost.test.ts's live-region
// tests.
test("DockChips shows a translated fallback for an id missing from meta, not the raw id", () => {
  const context = setAppContextForTest({ t: (k, p) => i18n.t(k, p) });
  render(DockChips, {
    props: { minimized: ["unregistered-id"], meta: new Map(), onRestore: () => {} },
    context,
  });

  const chip = screen.getByTestId("chip-unregistered-id");
  // The fallback legitimately embeds the id inside the translated message
  // (`panels.unknownPanel`) — the bug this guards against is the BARE raw
  // id standing in as the whole label, not the id's mere presence.
  expect(chip.textContent?.trim()).toBe("Unknown panel (unregistered-id)");
});
