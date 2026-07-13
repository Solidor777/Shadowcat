import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { chat } from "./index";

describe("chat module", () => {
  it("requires the panel surface and provides both singleton chat surfaces", () => {
    expect(chat.manifest.id).toBe("chat");
    expect(chat.manifest.requires).toContain(PANEL_CONTRACT);
    expect(chat.manifest.provides).toEqual([
      { contract: "shadowcat.surface:chat.composer", cardinality: "singleton" },
      { contract: "shadowcat.surface:chat.message", cardinality: "singleton" },
    ]);
  });

  it("contributes a default (order 0) docked-right panel with the chat icon", () => {
    const contributions = new ContributionRegistry();
    chat.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].order).toBe(0);
    expect(list[0].panel).toEqual({
      icon: "💬",
      labelKey: "chat.tab",
      defaultPlacement: { kind: "docked", zone: "right" },
    });
  });
});
