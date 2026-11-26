import type { Locator } from "@playwright/test";

/** A running record of the highest numeric value an element attribute has held since the latch
 * was installed. */
export interface AttributeMaxLatch {
  /** The highest finite numeric value the attribute has held (its value at install time
   * included); an absent or non-numeric value counts as 0.
   * @returns The latched maximum. */
  max(): Promise<number>;
}

/** Installs an in-page observer that latches the highest numeric value `attribute` reaches on
 * the element `locator` resolves to.
 *
 * Exists for values that rise for a bounded time and then fall on their own (a one-shot effect
 * counted while it plays): polling the live value can straddle the window between two polls and
 * read the resting value both times, so the assertion passes or fails on poll timing rather than
 * on whether the value ever rose. Every mutation record contributes both its previous and its
 * current value, so a rise that reverts before the observer callback runs is still recorded.
 * INVARIANT: install the latch BEFORE the action that raises the value; it records nothing that
 * happened earlier. The observer lives as long as the page.
 * @param locator - Resolves to exactly one element carrying `attribute`.
 * @param attribute - The attribute name, holding a decimal number.
 * @returns The latch; read it with `max()`.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const latch = await latchAttributeMax(page.locator(".stage-host"), "data-vfx-count");
 * await page.getByTestId("scene-tool-vfx").click();
 * const peak = await latch.max();
 * ```
 */
export async function latchAttributeMax(locator: Locator, attribute: string): Promise<AttributeMaxLatch> {
  const state = await locator.evaluateHandle((el, name) => {
    const numeric = (raw: string | null): number => {
      const v = raw === null ? 0 : Number(raw);
      return Number.isFinite(v) ? v : 0;
    };
    const latched = { max: numeric(el.getAttribute(name)) };
    const observer = new MutationObserver((records) => {
      for (const r of records) {
        latched.max = Math.max(latched.max, numeric(r.oldValue), numeric(el.getAttribute(name)));
      }
    });
    observer.observe(el, { attributes: true, attributeFilter: [name], attributeOldValue: true });
    return latched;
  }, attribute);
  return { max: () => state.evaluate((latched) => latched.max) };
}
