import { describe, it, expect } from "vitest";
import { CHAT_CARD_EXTENSION_CONTRACT, ContributionRegistry } from "@shadowcat/core";
import { chatCard } from "./index";

describe("chat-card module", () => {
  it("requires the chat.message surface and provides the multi-cardinality chat-card extension contract", () => {
    expect(chatCard.manifest.id).toBe("chat-card");
    expect(chatCard.manifest.requires).toContain("shadowcat.surface:chat.message");
    expect(chatCard.manifest.provides).toEqual([{ contract: CHAT_CARD_EXTENSION_CONTRACT, cardinality: "multi" }]);
  });

  it("contributes exactly one component to shadowcat.surface:chat.message", () => {
    const contributions = new ContributionRegistry();
    chatCard.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:chat.message");
    expect(list.length).toBe(1);
    expect(list[0].id).toBe("chat-card:main");
  });
});
