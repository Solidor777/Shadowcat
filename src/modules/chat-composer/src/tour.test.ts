import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Composer from "./Composer.svelte";
import { chatComposer } from "./index";
import { diceTour } from "./tour";

describe("dice tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    chatComposer.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["chat-composer:tour"]);
    expect(tours[0].tour).toBe(diceTour);
  });

  it("is offered on first use to everyone", () => {
    expect(diceTour.trigger).toBe("first-use");
    expect(diceTour.audience).toBe("any");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [diceTour.labelKey, ...diceTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors the composer mounts", () => {
    render(Composer, {
      props: { channel: "general", audience: { kind: "public" }, placeholderName: "general" },
      context: setAppContextForTest({ role: "player" }),
    });
    for (const step of diceTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });
});
