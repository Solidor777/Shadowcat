import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenDoc,
  buildTokenFromActor,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import TokenDispositionControl from "./TokenDispositionControl.svelte";

const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null as string | null, conditions: [], prototype: false,
  vision: null, light: null, movement: [],
  aura: null, sound: null, vfx: null,
};

const NO_OVERRIDES = { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null };

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A linked token against a one-actor store, with optional stored overrides. */
function linkedSetup(overrides?: TokenEngine["overrides"]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (overrides !== undefined) (token.engine as TokenEngine).overrides = overrides;
  return storeWith(actor, token);
}

describe("TokenDispositionControl", () => {
  it("renders nothing for a raw token, an instanced token, or when canEdit refuses", () => {
    const raw = buildTokenDoc("w1", "scene1", {
      x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false,
    }, "tok-raw");
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: storeWith(raw), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-raw" },
    });
    expect(screen.queryByTestId("token-disposition")).toBeNull();

    const instanced = buildTokenFromActor("w1", "scene1", buildActorDoc("w1", "G", actorEngine, "act1"), "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-i");
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: storeWith(instanced), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-i" },
    });
    expect(screen.queryByTestId("token-disposition")).toBeNull();

    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-disposition")).toBeNull();
  });

  it("shows inherit when no override is stored", () => {
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-disposition") as HTMLSelectElement).value).toBe("inherit");
  });

  it("choosing a stance writes the whole overrides object with the raw stored old (null on a fresh token)", async () => {
    const dispatchIntent = vi.fn();
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByTestId("token-disposition"), { target: { value: "friendly" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: null, new: { ...NO_OVERRIDES, disposition: "friendly" } }] },
    ]);
  });

  it("a stored stance reads back, and switching to inherit clears only disposition", async () => {
    const dispatchIntent = vi.fn();
    const stored = { ...NO_OVERRIDES, movement: ["flying"], disposition: "hostile" as const };
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: linkedSetup(stored), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const select = screen.getByTestId("token-disposition") as HTMLSelectElement;
    expect(select.value).toBe("hostile");
    await fireEvent.change(select, { target: { value: "inherit" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: stored, new: { ...stored, disposition: null } }] },
    ]);
  });

  it("re-choosing the stored stance dispatches nothing", async () => {
    const dispatchIntent = vi.fn();
    render(TokenDispositionControl, {
      context: setAppContextForTest({ documents: linkedSetup({ ...NO_OVERRIDES, disposition: "neutral" }), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByTestId("token-disposition"), { target: { value: "neutral" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});
