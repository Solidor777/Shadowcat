import { describe, it, expect, afterEach, vi } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { RollOutcome } from "@shadowcat/core";
import RollTooltip from "./RollTooltip.svelte";

/** Stubs `window.matchMedia("(hover: hover)")` for the touch-vs-mouse branch RollTooltip's
 * onClick reads. `hoverCapable: false` simulates a touch device (no mouseenter/focus-on-tap),
 * where a tap must toggle the popover open. */
function stubHoverCapable(hoverCapable: boolean): void {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query === "(hover: hover)" ? hoverCapable : false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** Mirrors dice::outcome::DieRecord's defaults, overridable per test (same shape as
 * `MessageCard.test`'s `dieRecord` — RollOutcome only cares about `kept`/`value` here). */
function dieRecord(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    value: 4, natural: 4, kept: true, exploded: false,
    crit_success: false, crit_fail: false, expertise: 0, group_index: 0,
    label: null, symbols: [],
    ...over,
  };
}

/** A localized `t` that resolves the keys this component renders (mirrors
 * `MessageCard.test`'s `fakeT`). */
function fakeT(key: string): string {
  const templates: Record<string, string> = {
    "chat.roll.details": "Roll details",
    "chat.roll.dropped": "dropped",
    "chat.roll.droppedBestOf": "lost to best",
    "chat.roll.naturalMax": "natural max",
    "chat.roll.naturalMin": "natural min",
  };
  return templates[key] ?? key;
}

/** Mirrors dice::outcome::RollOutcome's defaults, overridable per test. */
function testRollOutcome(over: Record<string, unknown> = {}): RollOutcome {
  return {
    total: 4, records: [dieRecord()],
    successes: null, pass: null, margin: null, tier_label: null, tier_value: null,
    crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    symbol_counts: {}, labeled_consts: [],
    ...over,
  } as unknown as RollOutcome;
}

describe("RollTooltip", () => {
  it("hovering/focusing the roll chip shows a popover with the full per-die table", async () => {
    const outcome = testRollOutcome({
      records: [dieRecord({ kept: true, value: 5 }), dieRecord({ kept: false, value: 2 }), dieRecord({ kept: true, value: 6 })],
    });
    const { getByRole, queryByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });

    expect(queryByRole("tooltip")).toBeNull(); // closed by default

    await fireEvent.focus(getByRole("button", { name: /roll details/i }));

    const tooltip = getByRole("tooltip");
    expect(tooltip.textContent).toContain("5");
    expect(tooltip.textContent).toContain("2");
    expect(tooltip.textContent).toContain("6");
    // Dropped (kept: false) dice should be visually/semantically distinguished, not just listed identically.
    expect(tooltip.querySelector('[data-dropped="true"]')?.textContent).toContain("2");
  });

  it("the popover is keyboard-accessible (focus opens, Escape closes)", async () => {
    const outcome = testRollOutcome({ records: [dieRecord({ kept: true, value: 4 })] });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });

    const trigger = getByRole("button", { name: /roll details/i });
    await fireEvent.focus(trigger);
    expect(getByRole("tooltip")).toBeTruthy();

    // fireEvent.focus dispatches a synthetic focus event without moving real DOM
    // focus (jsdom), so the Escape keydown targets the trigger directly rather
    // than `document.activeElement`.
    await fireEvent.keyDown(trigger, { key: "Escape" });
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
  });

  it("tapping the chip on a touch device (no hover) toggles the popover open, then closed", async () => {
    stubHoverCapable(false);
    const outcome = testRollOutcome({ records: [dieRecord({ kept: true, value: 3 })] });
    const { getByRole, queryByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });

    const trigger = getByRole("button", { name: /roll details/i });
    await fireEvent.click(trigger);
    expect(getByRole("tooltip")).toBeTruthy();

    await fireEvent.click(trigger);
    expect(queryByRole("tooltip")).toBeNull();
  });

  it("clicking the chip on a hover-capable (mouse) device does not toggle it closed", async () => {
    stubHoverCapable(true);
    const outcome = testRollOutcome({ records: [dieRecord({ kept: true, value: 3 })] });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });

    const trigger = getByRole("button", { name: /roll details/i });
    await fireEvent.mouseEnter(trigger);
    expect(getByRole("tooltip")).toBeTruthy();

    // Hover already opened it; a click on a hover-capable device must not toggle it closed
    // (mouseenter fires before click, so a naive toggle would immediately re-close it).
    await fireEvent.click(trigger);
    expect(getByRole("tooltip")).toBeTruthy();
  });

  it("a document-level Escape dismisses a hover-opened (not focused) popover", async () => {
    stubHoverCapable(true);
    const outcome = testRollOutcome({ records: [dieRecord({ kept: true, value: 3 })] });
    const { getByRole, queryByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });

    const trigger = getByRole("button", { name: /roll details/i });
    await fireEvent.mouseEnter(trigger);
    expect(getByRole("tooltip")).toBeTruthy();

    await fireEvent.keyDown(document, { key: "Escape" });
    expect(queryByRole("tooltip")).toBeNull();
  });

  it("two mounted RollTooltips never produce duplicate popover ids", async () => {
    const outcomeA = testRollOutcome({ records: [dieRecord({ value: 1 })] });
    const outcomeB = testRollOutcome({ records: [dieRecord({ value: 2 })] });
    const context = setAppContextForTest({ t: fakeT });
    const { getAllByRole } = render(RollTooltip, { props: { outcome: outcomeA }, context });
    render(RollTooltip, { props: { outcome: outcomeB }, context });

    const triggers = getAllByRole("button", { name: /roll details/i });
    expect(triggers).toHaveLength(2);
    await fireEvent.focus(triggers[0]);
    await fireEvent.focus(triggers[1]);

    const tooltips = document.querySelectorAll('[role="tooltip"]');
    expect(tooltips).toHaveLength(2);
    expect(tooltips[0].id).not.toBe(tooltips[1].id);
    expect(tooltips[0].id).not.toBe("");
    expect(tooltips[1].id).not.toBe("");
  });

  it("marks a Numeric die's natural max and min in a bare Total roll, and no other die", async () => {
    const numeric = { Numeric: { min: 1, max: 20 } };
    const outcome = testRollOutcome({
      crit_successes: 0,
      records: [
        dieRecord({ natural: 20, value: 20, kind: numeric }),
        dieRecord({ natural: 1, value: 1, kind: numeric }),
        dieRecord({ natural: 10, value: 10, kind: numeric }),
      ],
    });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });
    await fireEvent.focus(getByRole("button", { name: /roll details/i }));

    const rows = Array.from(document.querySelectorAll("tbody tr"));
    expect(rows.map((r) => r.getAttribute("data-nat-extreme"))).toEqual(["max", "min", null]);
    expect(rows[0].textContent).toContain("natural max");
    expect(rows[1].textContent).toContain("natural min");
  });

  it("never marks a Faces die or a record with no kind, whatever its natural", async () => {
    const faces = { Faces: { faces: [{ value: 1, symbols: [] }, { value: 20, symbols: [] }] } };
    const outcome = testRollOutcome({
      records: [
        dieRecord({ natural: 20, kind: faces }),
        dieRecord({ natural: 0, kind: faces }),
        dieRecord({ natural: 20, kind: null }),
        dieRecord({ natural: 20 }),
      ],
    });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });
    await fireEvent.focus(getByRole("button", { name: /roll details/i }));
    expect(document.querySelectorAll("[data-nat-extreme='min'], [data-nat-extreme='max']")).toHaveLength(0);
  });

  it("a die discarded by a best(...) branch gives its own reason, an ordinary drop the generic one", async () => {
    const outcome = testRollOutcome({
      records: [
        dieRecord({ kept: false, value: 2, best_of_branch: 1 }),
        dieRecord({ kept: false, value: 3, best_of_branch: null }),
      ],
    });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });
    await fireEvent.focus(getByRole("button", { name: /roll details/i }));
    const tags = [...document.querySelectorAll("tbody tr .dropped-tag")].map((e) => e.textContent?.trim());
    expect(tags).toEqual(["lost to best", "dropped"]);
  });

  it("a dropped natural max carries both the dropped and the natural-extreme marks", async () => {
    const outcome = testRollOutcome({
      records: [dieRecord({ natural: 6, kept: false, kind: { Numeric: { min: 1, max: 6 } } })],
    });
    const { getByRole } = render(RollTooltip, {
      props: { outcome },
      context: setAppContextForTest({ t: fakeT }),
    });
    await fireEvent.focus(getByRole("button", { name: /roll details/i }));
    const row = document.querySelector("tbody tr");
    expect(row?.getAttribute("data-dropped")).toBe("true");
    expect(row?.getAttribute("data-nat-extreme")).toBe("max");
  });

  it("renders a recalculated badge when recalcHistory is non-empty", () => {
    const { container } = render(RollTooltip, {
      props: {
        outcome: testRollOutcome(),
        recalcHistory: [{ previous_outcome: testRollOutcome({ total: 2 }), recalculated_by: "u-gm", recalculated_at: 100 }],
      },
      context: setAppContextForTest({ t: fakeT }),
    });
    expect(container.querySelector(".chip.recalculated")).not.toBeNull();
  });

  it("renders no badge when recalcHistory is absent", () => {
    const { container } = render(RollTooltip, {
      props: { outcome: testRollOutcome() },
      context: setAppContextForTest({ t: fakeT }),
    });
    expect(container.querySelector(".chip.recalculated")).toBeNull();
  });
});
