import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { chat } from "./index";

describe("chat module", () => {
  it("requires the sidebar surface and provides both singleton chat surfaces", () => {
    expect(chat.manifest.id).toBe("chat");
    expect(chat.manifest.requires).toContain("shadowcat.surface:sidebar");
    expect(chat.manifest.provides).toEqual([
      { contract: "shadowcat.surface:chat.composer", cardinality: "singleton" },
      { contract: "shadowcat.surface:chat.message", cardinality: "singleton" },
    ]);
  });

  it("contributes a default (order 0) sidebar tab with the chat icon", () => {
    const contributions = new ContributionRegistry();
    chat.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:sidebar");
    expect(list.length).toBe(1);
    expect(list[0].order).toBe(0);
    expect(list[0].tab).toEqual({ icon: "💬", labelKey: "chat.tab" });
  });
});
