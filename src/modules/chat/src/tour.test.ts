import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, DocumentStore, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ChatPanel from "./ChatPanel.svelte";
import { chat } from "./index";
import { chatTour } from "./tour";

describe("chat tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    chat.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["chat:tour"]);
    expect(tours[0].tour).toBe(chatTour);
  });

  it("is offered on first use to everyone", () => {
    expect(chatTour.trigger).toBe("first-use");
    expect(chatTour.audience).toBe("any");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [chatTour.labelKey, ...chatTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors the chat panel mounts", () => {
    const documents = new DocumentStore();
    render(ChatPanel, {
      context: setAppContextForTest({ role: "player", world: "w1", documents, store: documents, contributions: new ContributionRegistry() }),
    });
    for (const step of chatTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });
});
