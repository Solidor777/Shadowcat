import { describe, it, expect } from "vitest";
import { render, fireEvent, waitFor, type RenderResult } from "@testing-library/svelte";
import { SessionPausedError, type PortalOfferNotice } from "@shadowcat/core";
import PortalChooser from "./PortalChooser.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";

/** One offer with a revealed name, a hidden name and an unavailable destination. */
function offer(over: Partial<PortalOfferNotice> = {}): PortalOfferNotice {
  return {
    offerId: "o1",
    scene: "s1",
    token: "t1",
    prompt: null,
    options: [
      { index: 0, label: "The Library", destination_name: "Hall of Maps", available: true },
      { index: 1, label: "Somewhere unknown", destination_name: null, available: true },
      { index: 2, label: "The Cellar", destination_name: null, available: false },
    ],
    gmOnly: false,
    owner: "u1",
    ...over,
  };
}

/** Mount the chooser over a recording portal seam; `push` delivers an offer as the server would. */
function mount(over: { choose?: (id: string, c: number) => Promise<void>; decline?: (id: string) => Promise<void>; notify?: (m: string) => void } = {}): {
  /** Delivers one offer to the mounted chooser. */
  push: (o: PortalOfferNotice) => void;
  /** Every `choose` call as `[offerId, choice]`. */
  chosen: [string, number][];
  /** Every declined offer id. */
  declined: string[];
  /** Every notification text. */
  notes: string[];
  /** The render result. */
  view: RenderResult<typeof PortalChooser>;
} {
  let deliver: (o: PortalOfferNotice) => void = () => {};
  const chosen: [string, number][] = [];
  const declined: string[] = [];
  const notes: string[] = [];
  const context = setAppContextForTest({
    portal: {
      onOffer: (cb) => {
        deliver = cb;
        return () => {};
      },
      choose: (id, c) => {
        chosen.push([id, c]);
        return over.choose?.(id, c) ?? Promise.resolve();
      },
      decline: (id) => {
        declined.push(id);
        return over.decline?.(id) ?? Promise.resolve();
      },
      open: () => Promise.resolve(),
    },
    notify: (m) => notes.push(m),
  });
  const view = render(PortalChooser, { context });
  return { push: (o) => deliver(o), chosen, declined, notes, view };
}

describe("PortalChooser", () => {
  it("renders nothing until an offer arrives", () => {
    const { view } = mount();
    expect(view.queryByTestId("portal-chooser")).toBeNull();
  });

  it("lists each option's label, shows a destination name only when one arrived, and disables an unavailable option", async () => {
    const { push, view } = mount();
    push(offer());
    await waitFor(() => expect(view.getByTestId("portal-chooser")).toBeTruthy());
    const options = view.getAllByTestId("portal-option") as HTMLButtonElement[];
    expect(options.map((o) => o.querySelector(".label")?.textContent)).toEqual(["The Library", "Somewhere unknown", "The Cellar"]);
    expect(view.getAllByTestId("portal-destination").map((n) => n.textContent)).toEqual(["Hall of Maps"]);
    expect(options.map((o) => o.disabled)).toEqual([false, false, true]);
  });

  it("uses the authored prompt as the heading, else the default title", async () => {
    const { push, view } = mount();
    push(offer({ prompt: "Where to?" }));
    await waitFor(() => expect(view.getByText("Where to?")).toBeTruthy());
    push(offer({ token: "t2", offerId: "o2", prompt: null }));
    // Only the first pending offer shows; the second waits.
    await waitFor(() => expect(view.getByText("portal.more")).toBeTruthy());
  });

  it("clicking an option sends its index and closes the chooser", async () => {
    const { push, chosen, view } = mount();
    push(offer());
    const options = await waitFor(() => view.getAllByTestId("portal-option"));
    await fireEvent.click(options[1]);
    expect(chosen).toEqual([["o1", 1]]);
    await waitFor(() => expect(view.queryByTestId("portal-chooser")).toBeNull());
  });

  it("Stay here declines the offer", async () => {
    const { push, declined, view } = mount();
    push(offer());
    await fireEvent.click(await waitFor(() => view.getByTestId("portal-decline")));
    expect(declined).toEqual(["o1"]);
    await waitFor(() => expect(view.queryByTestId("portal-chooser")).toBeNull());
  });

  it("Escape declines the offer", async () => {
    const { push, declined, view } = mount();
    push(offer());
    await waitFor(() => view.getByTestId("portal-chooser"));
    await fireEvent.keyDown(document, { key: "Escape" });
    expect(declined).toEqual(["o1"]);
    await waitFor(() => expect(view.queryByTestId("portal-chooser")).toBeNull());
  });

  it("focuses the first available option and moves focus with the arrow keys", async () => {
    const { push, view } = mount();
    push(offer());
    const options = (await waitFor(() => view.getAllByTestId("portal-option"))) as HTMLButtonElement[];
    await waitFor(() => expect(document.activeElement).toBe(options[0]));
    await fireEvent.keyDown(view.getByTestId("portal-chooser"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(options[1]);
    // The unavailable third option is skipped: ArrowDown wraps to the first.
    await fireEvent.keyDown(view.getByTestId("portal-chooser"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(options[0]);
    await fireEvent.keyDown(view.getByTestId("portal-chooser"), { key: "ArrowUp" });
    expect(document.activeElement).toBe(options[1]);
  });

  it("a newer offer for the same token replaces the older one", async () => {
    const { push, chosen, view } = mount();
    push(offer({ offerId: "old" }));
    push(offer({ offerId: "new" }));
    await fireEvent.click((await waitFor(() => view.getAllByTestId("portal-option")))[0]);
    expect(chosen).toEqual([["new", 0]]);
    await waitFor(() => expect(view.queryByTestId("portal-chooser")).toBeNull());
  });

  it("shows the server's refusal text in a notification", async () => {
    const { push, notes, view } = mount({ choose: () => Promise.reject(new Error("portal rejected")) });
    push(offer());
    await fireEvent.click((await waitFor(() => view.getAllByTestId("portal-option")))[0]);
    await waitFor(() => expect(notes).toEqual(["portal rejected"]));
  });

  it("shows the paused notice when the session pause refused the answer", async () => {
    const { push, notes, view } = mount({ choose: () => Promise.reject(new SessionPausedError("paused")) });
    push(offer());
    await fireEvent.click((await waitFor(() => view.getAllByTestId("portal-option")))[0]);
    await waitFor(() => expect(notes).toEqual(["intent.rejected.paused"]));
  });

  it("uses the card layout when the viewport is expanded", async () => {
    const { push, view } = mount();
    push(offer());
    const dialog = await waitFor(() => view.getByTestId("portal-chooser"));
    expect(dialog.getAttribute("data-layout")).toBe("card");
    expect(dialog.getAttribute("aria-modal")).toBeNull();
  });
});
