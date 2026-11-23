import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, DocumentStore, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NotesPanel from "./NotesPanel.svelte";
import { notes } from "./index";
import { notesTour } from "./tour";

describe("notes tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    notes.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["notes:tour"]);
    expect(tours[0].tour).toBe(notesTour);
  });

  it("is replay-only, because the panel is closed until the user opens it", () => {
    expect(notesTour.trigger).toBe("manual");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [notesTour.labelKey, ...notesTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors the notes panel mounts", () => {
    render(NotesPanel, { context: setAppContextForTest({ documents: new DocumentStore(), role: "player" }) });
    for (const step of notesTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });
});
