import type { Locator, Page } from "@playwright/test";

/** A running record of every value an element attribute has held since the latch was installed. */
export interface AttributeValuesLatch {
  /** The distinct values the attribute has held, in first-seen order; the value at install time
   * is first, and an absent attribute is recorded as `null`.
   * @returns A copy of the recorded values. */
  seen(): Promise<(string | null)[]>;
  /** Every value the attribute has taken, in order and REPEATS INCLUDED: the value at install time,
   * then one entry per mutation record, so its length grows on every write even when the value
   * has been seen before. Use it to prove that NO write happened across an interval.
   * @returns A copy of the ordered history. */
  transitions(): Promise<(string | null)[]>;
}

/** Installs an in-page observer that records every value `attribute` takes on the element
 * `locator` resolves to.
 *
 * Exists for a NON-numeric state that is held for a bounded time and then moves on by itself (a
 * tumbling/settled/idle machine, a mode that flips back): polling the live value can straddle the
 * window and read only the resting value, so the assertion depends on poll timing rather than on
 * whether the state was ever entered. `latchAttributeMax` covers the numeric case. Every mutation
 * record contributes its previous and its current value. INVARIANT: install the latch BEFORE the
 * action that moves the state; it records nothing that happened earlier.
 * @param locator - Resolves to exactly one element.
 * @param attribute - The attribute name.
 * @returns The latch; read it with `seen()`.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const latch = await latchAttributeValues(page.locator(".dice3d-overlay"), "data-dice3d-state");
 * await page.getByRole("button", { name: "Roll" }).click();
 * const states = await latch.seen();
 * ```
 */
export async function latchAttributeValues(locator: Locator, attribute: string): Promise<AttributeValuesLatch> {
  const state = await locator.evaluateHandle((el, name) => {
    const values: (string | null)[] = [el.getAttribute(name)];
    const log: (string | null)[] = [el.getAttribute(name)];
    const note = (v: string | null): void => {
      if (!values.includes(v)) values.push(v);
    };
    const observer = new MutationObserver((records) => {
      records.forEach((r, i) => {
        note(r.oldValue);
        // Record k's old value is the value written by record k-1, so the whole history is recoverable
        // even when several writes land in one batch.
        if (i > 0) log.push(r.oldValue);
      });
      const current = el.getAttribute(name);
      note(current);
      if (records.length > 0) log.push(current);
    });
    observer.observe(el, { attributes: true, attributeFilter: [name], attributeOldValue: true });
    return { values, log };
  }, attribute);
  return {
    seen: () => state.evaluate((s) => [...s.values]),
    transitions: () => state.evaluate((s) => [...s.log]),
  };
}

/** A record of whether a piece of text has ever been present in a page's document. */
export interface TextSeenLatch {
  /** Whether the text has been in the document at any moment since the latch was installed
   * (including at install time).
   * @returns `true` once seen; never reverts to `false`. */
  seen(): Promise<boolean>;
}

/** Installs an in-page observer that records whether `text` ever appears in the document body.
 *
 * Exists for a notice that removes itself (an auto-dismissing toast): a polled visibility
 * assertion can straddle its lifetime and read it absent both times. The observer inspects the
 * nodes each mutation record added, so a node that is added and removed inside one batch of
 * mutations is still recorded. BOUND: text written into an EXISTING text node and overwritten again
 * inside one batch is not recoverable, because a character-data record carries only the previous
 * value; such a change is read at callback time. INVARIANT: install the latch
 * BEFORE the action that raises the text; it records nothing that happened earlier. The observer
 * lives as long as the page.
 * @param page - The page whose document is watched.
 * @param text - The substring to watch for in the body's text content.
 * @returns The latch; read it with `seen()`.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const toast = await latchTextSeen(page, "Your turn");
 * await page.getByRole("button", { name: "Advance" }).click();
 * const appeared = await toast.seen();
 * ```
 */
export async function latchTextSeen(page: Page, text: string): Promise<TextSeenLatch> {
  const state = await page.evaluateHandle((needle) => {
    const latched = { seen: document.body.textContent?.includes(needle) ?? false };
    const observer = new MutationObserver((records) => {
      if (latched.seen) return;
      for (const r of records) {
        if (r.type === "characterData" && (r.target.textContent?.includes(needle) ?? false)) latched.seen = true;
        for (const added of r.addedNodes) if (added.textContent?.includes(needle)) latched.seen = true;
      }
      if (!latched.seen && (document.body.textContent?.includes(needle) ?? false)) latched.seen = true;
    });
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    return latched;
  }, text);
  return { seen: () => state.evaluate((latched) => latched.seen) };
}
