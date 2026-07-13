import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { chatComposer } from "./index";

describe("chat-composer module", () => {
  it("requires the chat.composer surface and provides nothing", () => {
    expect(chatComposer.manifest.id).toBe("chat-composer");
    expect(chatComposer.manifest.requires).toEqual(["shadowcat.surface:chat.composer"]);
    expect(chatComposer.manifest.provides).toEqual([]);
  });

  it("contributes chat-composer:main into the chat.composer surface", () => {
    const contributions = new ContributionRegistry();
    chatComposer.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:chat.composer");
    expect(list.length).toBe(1);
    expect(list[0].id).toBe("chat-composer:main");
  });
});
