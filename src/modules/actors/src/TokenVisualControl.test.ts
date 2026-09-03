import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, buildTokenDoc, type TokenEngine, type TokenVisual, type WireDocument, type WireOperation } from "@shadowcat/core";
import TokenVisualControl from "./TokenVisualControl.svelte";

const cmd = (ops: WireOperation[]) => ({
  seq: 1,
  world_id: "w1",
  author: "a",
  ts: 0,
  ops,
});
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

/** A token engine body; `linked: true` sets an actor_id (overrides project only for a linked
 * token — the control hides for instanced/raw tokens). Same fixture shape as
 * `TokenEmissionControl`'s suite. */
function tokenEngine(linked: boolean, overrides: TokenEngine["overrides"] = null): TokenEngine {
  return {
    x: 0,
    y: 0,
    w: 100,
    h: 100,
    rotation: 0,
    visual: null,
    elevation: null,
    actor_id: linked ? "act1" : null,
    overrides,
    face: null,
  };
}

function actorDoc(visual: TokenVisual = { kind: "image", asset: "a1" }): WireDocument {
  return buildActorDoc(
    "w1",
    "Troll",
    { displayName: "Troll", visual, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null , light: null, movement: [] },
    "act1",
  );
}

const assets = { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never;

describe("TokenVisualControl", () => {
  it("renders nothing without a selected token", () => {
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true), "tok1")), dispatchIntent: vi.fn(), canEdit: () => true, assets }),
      props: { tokenId: null },
    });
    expect(screen.queryByText("actors.tokenVisualHint")).toBeNull();
  });

  it("renders nothing for an instanced (unlinked) token — its overrides would be inert", () => {
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(buildTokenDoc("w1", "scene1", tokenEngine(false), "tok1")), dispatchIntent: vi.fn(), canEdit: () => true, assets }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByText("actors.tokenVisualHint")).toBeNull();
  });

  it("renders nothing when canEdit refuses", () => {
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true), "tok1")), dispatchIntent: vi.fn(), canEdit: () => false, assets }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByText("actors.tokenVisualHint")).toBeNull();
  });

  it("initializes the editor from the token's effective (actor-inherited) visual", () => {
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true), "tok1")), dispatchIntent: vi.fn(), canEdit: () => true, assets }),
      props: { tokenId: "tok1" },
    });
    expect(screen.getByText("actors.tokenVisualHint")).toBeTruthy();
    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("image");
    // No stored override yet → no clear affordance.
    expect(screen.queryByText("actors.clearVisualOverride")).toBeNull();
  });

  it("apply dispatches /engine/overrides/visual with old:null when no override exists", async () => {
    const dispatchIntent = vi.fn();
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true), "tok1")), dispatchIntent, canEdit: () => true, assets }),
      props: { tokenId: "tok1" },
    });
    const apply = screen.getByText("actors.applyVisual") as HTMLButtonElement;
    await vi.waitFor(() => expect(apply.disabled).toBe(false));
    await fireEvent.click(apply);
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides/visual", old: null, new: { kind: "image", asset: "a1" } }] },
    ]);
  });

  it("apply reads the RAW stored override as the OCC pre-image", async () => {
    const dispatchIntent = vi.fn();
    const existing: TokenVisual = { kind: "image", asset: "ov1" };
    const overrides = { name: null, visual: existing, size: null, shape: null, vision: null, aura: null, sound: null, vfx: null , light: null, movement: null };
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true, overrides), "tok1")), dispatchIntent, canEdit: () => true, assets }),
      props: { tokenId: "tok1" },
    });
    // The effective visual IS the stored override (resolveTokenActor folds it over the actor base).
    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("image");
    const apply = screen.getByText("actors.applyVisual") as HTMLButtonElement;
    await vi.waitFor(() => expect(apply.disabled).toBe(false));
    await fireEvent.click(apply);
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides/visual", old: existing, new: existing }] },
    ]);
  });

  it("clear dispatches /engine/overrides/visual writing null with the raw stored override as `old`", async () => {
    const dispatchIntent = vi.fn();
    const existing: TokenVisual = { kind: "image", asset: "ov1" };
    const overrides = { name: null, visual: existing, size: null, shape: null, vision: null, aura: null, sound: null, vfx: null , light: null, movement: null };
    render(TokenVisualControl, {
      context: setAppContextForTest({ documents: storeWith(actorDoc(), buildTokenDoc("w1", "scene1", tokenEngine(true, overrides), "tok1")), dispatchIntent, canEdit: () => true, assets }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.click(screen.getByText("actors.clearVisualOverride"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides/visual", old: existing, new: null }] },
    ]);
  });
});
