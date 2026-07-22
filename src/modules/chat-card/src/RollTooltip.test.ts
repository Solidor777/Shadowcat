import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { RollOutcome } from "@shadowcat/core";
import RollTooltip from "./RollTooltip.svelte";

/** Mirrors dice::outcome::DieRecord's defaults, overridable per test (same shape as
 * MessageCard.test.ts's local fixture — RollOutcome only cares about `kept`/`value` here). */
function dieRecord(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    value: 4, natural: 4, kept: true, exploded: false,
    crit_success: false, crit_fail: false, expertise: 0, group_index: 0,
    label: null, symbols: [],
    ...over,
  };
}

/** A localized `t` that resolves the keys this component renders (mirrors
 * MessageCard.test.ts's `fakeT`). */
function fakeT(key: string): string {
  const templates: Record<string, string> = {
    "chat.roll.details": "Roll details",
    "chat.roll.dropped": "dropped",
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
});
