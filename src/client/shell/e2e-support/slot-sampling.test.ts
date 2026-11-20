// @vitest-environment node
import { describe, expect, it } from "vitest";
import { sampleSlots } from "./slot-sampling";

describe("sampleSlots", () => {
  it("checks the run after every slot", async () => {
    const order: string[] = [];
    await sampleSlots(
      3,
      async (slot) => {
        order.push(`read ${slot}`);
      },
      () => {
        order.push("check");
      },
    );
    expect(order).toEqual(["read 0", "check", "read 1", "check", "read 2", "check"]);
  });

  it("reports a violation at the first slot that shows it and reads no later slot", async () => {
    const read: number[] = [];
    let oversizedAt: number | null = null;
    const run = sampleSlots(
      120,
      async (slot) => {
        read.push(slot);
        if (slot === 3) oversizedAt = slot;
      },
      () => {
        if (oversizedAt !== null) throw new Error(`slice over the cap seen at slot ${oversizedAt}`);
      },
    );
    await expect(run).rejects.toThrow("slice over the cap seen at slot 3");
    expect(read).toEqual([0, 1, 2, 3]);
  });
});
