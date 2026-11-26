import { test, expect, vi } from "vitest";
import { render, waitFor } from "@testing-library/svelte";
import type { PortalOfferNotice } from "@shadowcat/core";
import { setAppContextForTest } from "./__fixtures__/appContextTest";

// `sizeClass()` reads `matchMedia` once at module load, so the stub must exist before the
// component (and with it `sizeClass`) is imported.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }));
const { default: PortalChooser } = await import("./PortalChooser.svelte");

test("a compact viewport shows the chooser as a blocking modal", async () => {
  let deliver: (o: PortalOfferNotice) => void = () => {};
  const context = setAppContextForTest({
    portal: {
      onOffer: (cb) => {
        deliver = cb;
        return () => {};
      },
      choose: () => Promise.resolve(),
      decline: () => Promise.resolve(),
      open: () => Promise.resolve(),
    },
  });
  const view = render(PortalChooser, { context });
  deliver({
    offerId: "o1",
    scene: "s1",
    token: "t1",
    prompt: null,
    options: [{ index: 0, label: "The Library", destination_name: null, available: true }],
    gmOnly: false,
    owner: null,
  });
  const dialog = await waitFor(() => view.getByTestId("portal-chooser"));
  expect(dialog.getAttribute("data-layout")).toBe("modal");
  expect(dialog.getAttribute("aria-modal")).toBe("true");
});
