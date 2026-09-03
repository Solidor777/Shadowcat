import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, buildTokenFromActor, type WireDocument, type WireOperation } from "@shadowcat/core";
import FaceSwapPalette from "./FaceSwapPalette.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

function facesActor(): WireDocument {
  return buildActorDoc(
    "w1",
    "Goblin",
    { displayName: "Goblin", visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal", faceMap: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
}

describe("FaceSwapPalette", () => {
  it("renders nothing when tokenId is null", () => {
    render(FaceSwapPalette, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(facesActor()), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: null },
    });
    expect(screen.queryByText("actors.faceSwapHint")).toBeNull();
  });

  it("renders the face options for a token whose effective visual is 'faces'", () => {
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    render(FaceSwapPalette, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(actor, token), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect(screen.getByText("actors.faceSwapHint")).toBeTruthy();
    expect(screen.getByRole("button", { name: "normal" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bloodied" })).toBeTruthy();
  });

  it("clicking a face dispatches an /engine/face update reading the raw stored value for `old`", async () => {
    const dispatchIntent = vi.fn();
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    render(FaceSwapPalette, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(actor, token), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "bloodied" }));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/face", old: null, new: "bloodied" }] },
    ]);
  });
});
