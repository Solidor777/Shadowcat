import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenDoc,
  buildTokenFromActor,
  type ActorEngine,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import TokenFacingVisionControl from "./TokenFacingVisionControl.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

const ACTOR_ENGINE: ActorEngine = {
  displayName: "Guard", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null,
  conditions: [], prototype: true, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null,
  facing_vision: { arc: 120, near: 1 },
};

/** A linked token on `actor1` with an optional stored override. */
function linkedToken(facing?: { arc: number; near: number }): WireDocument {
  const actor = buildActorDoc("w1", "Guard", ACTOR_ENGINE, "actor1");
  const tok = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (facing) {
    (tok.engine as TokenEngine).overrides = { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null, facing_vision: facing };
  }
  return tok;
}

function rawToken(facing?: { arc: number; near: number }): WireDocument {
  const engine: TokenEngine = {
    x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null,
    elevation: null, lock_rotation: false, identity_hidden: false, ...(facing ? { facing_vision: facing } : {}),
  };
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

const actorDoc = buildActorDoc("w1", "Guard", ACTOR_ENGINE, "actor1");

function mount(doc: WireDocument, over: { role?: "gm" | "player"; canEdit?: () => boolean } = {}) {
  const dispatchIntent = vi.fn();
  render(TokenFacingVisionControl, {
    context: setAppContextForTest({ role: over.role ?? "gm", documents: storeWith(actorDoc, doc), dispatchIntent, canEdit: over.canEdit ?? (() => true) }),
    props: { tokenId: "tok1" },
  });
  return dispatchIntent;
}

describe("TokenFacingVisionControl", () => {
  it("renders nothing for a player, without a selection, or when canEdit refuses", () => {
    mount(linkedToken(), { role: "player" });
    expect(screen.queryByTestId("token-facing-mode")).toBeNull();
  });

  it("renders nothing for an instanced token", () => {
    const instanced = buildTokenFromActor("w1", "scene1", actorDoc, "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    mount(instanced);
    expect(screen.queryByTestId("token-facing-mode")).toBeNull();
    expect(screen.queryByTestId("facing-arc")).toBeNull();
  });

  it("a linked token inherits by default and custom seeds from the actor's value in a whole overrides write", async () => {
    const dispatchIntent = mount(linkedToken());
    const mode = screen.getByTestId("token-facing-mode") as HTMLSelectElement;
    expect(mode.value).toBe("inherit");
    expect(screen.queryByTestId("facing-arc")).toBeNull();
    await fireEvent.change(mode, { target: { value: "custom" } });
    const call = dispatchIntent.mock.calls[0][0][0];
    expect(call.changes[0].path).toBe("/engine/overrides");
    expect(call.changes[0].old).toBeNull();
    expect(call.changes[0].new.facing_vision).toEqual({ arc: 120, near: 1 });
  });

  it("a linked token with an override shows custom, edits the arc, and inherit drops the override", async () => {
    const dispatchIntent = mount(linkedToken({ arc: 60, near: 0 }));
    expect((screen.getByTestId("token-facing-mode") as HTMLSelectElement).value).toBe("custom");
    expect((screen.getByTestId("facing-arc") as HTMLInputElement).value).toBe("60");
    expect(screen.queryByTestId("facing-enabled")).toBeNull();

    await fireEvent.change(screen.getByTestId("facing-arc"), { target: { value: "90" } });
    expect(dispatchIntent.mock.calls[0][0][0].changes[0].new.facing_vision).toEqual({ arc: 90, near: 0 });

    await fireEvent.change(screen.getByTestId("token-facing-mode"), { target: { value: "inherit" } });
    const cleared = dispatchIntent.mock.calls[1][0][0].changes[0];
    expect(cleared.old.facing_vision).toEqual({ arc: 60, near: 0 });
    expect(cleared.new.facing_vision).toBeUndefined();
  });

  it("an actorless token edits its own /engine/facing_vision with the raw stored old", async () => {
    const dispatchIntent = mount(rawToken());
    expect(screen.queryByTestId("token-facing-mode")).toBeNull();
    await fireEvent.click(screen.getByTestId("facing-enabled"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/facing_vision", old: null, new: { arc: 120, near: 1 } }] },
    ]);
  });

  it("an actorless token with a stored value clears it to null", async () => {
    const dispatchIntent = mount(rawToken({ arc: 45, near: 0 }));
    await fireEvent.click(screen.getByTestId("facing-enabled"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/facing_vision", old: { arc: 45, near: 0 }, new: null }] },
    ]);
  });
});
