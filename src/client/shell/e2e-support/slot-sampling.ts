/**
 * Reads a long run one slot at a time and checks the run's invariants after every slot.
 *
 * `check` runs after each `read`, so a violation it finds is reported after the slot whose traffic
 * first showed it and no later slot is read. A check deferred to the end of the run would report
 * the same violation only after the whole run had played.
 * @param slots - The number of slots in the run.
 * @param read - Waits for slot `slot` and records its reading.
 * @param check - Throws when any invariant of the run so far is violated.
 * @returns Resolves after the last slot is read and checked.
 * @example
 * ```
 * const seen: number[] = [];
 * await sampleSlots(3, async (slot) => { seen.push(slot); }, () => {});
 * ```
 */
export async function sampleSlots(
  slots: number,
  read: (slot: number) => Promise<void>,
  check: () => void,
): Promise<void> {
  for (let slot = 0; slot < slots; slot++) {
    await read(slot);
    check();
  }
}
