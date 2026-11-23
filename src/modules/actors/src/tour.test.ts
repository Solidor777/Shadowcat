import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, DocumentStore, TOUR_CONTRACT, buildActorDoc, buildConditionRegistryDoc, buildTokenFromActor, type WireDocument } from "@shadowcat/core";
import { TokenSelection, findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ActorsPanel from "./ActorsPanel.svelte";
import TokenQuickEdit from "./TokenQuickEdit.svelte";
import { actors } from "./index";
import { actorsTour } from "./tour";

// The panel's emission editor lists assets on mount; jsdom has no server to answer.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return { ...actual, listAssets: vi.fn().mockResolvedValue([]) };
});

/** A store holding one linked token with its actor and a condition registry. */
function tokenStore(): DocumentStore {
  const actor = buildActorDoc("w1", "G", { displayName: "G", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null, bars: [] }, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "x" } }, "creg1");
  const store = new DocumentStore();
  const docs: WireDocument[] = [actor, token, registry];
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return store;
}

describe("actors tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    actors.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["actors:tour"]);
    expect(tours[0].tour).toBe(actorsTour);
  });

  it("is replay-only, because the panel is closed until the user opens it", () => {
    expect(actorsTour.trigger).toBe("manual");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [actorsTour.labelKey, ...actorsTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets the anchors the panel mounts, leaving the quick-edit step to a selected token", () => {
    render(ActorsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore() }) });
    const mounted = actorsTour.steps.filter((s) => findTourAnchor(document, s.target) !== null).map((s) => s.target);
    expect(mounted).toEqual(["actors.panel", "actors.search", "actors.list"]);
  });

  it("mounts the quick-edit anchor once a token is selected", () => {
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(TokenQuickEdit, { context: setAppContextForTest({ documents: tokenStore(), tokenSelection, canEdit: () => true }), props: { tokenId: "tok1" } });
    expect(findTourAnchor(document, "actors.quick-edit")).not.toBeNull();
  });
});
