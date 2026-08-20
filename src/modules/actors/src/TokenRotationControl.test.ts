import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildTokenDoc,
  type TokenEngine,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import TokenRotationControl from "./TokenRotationControl.svelte";

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

function tokenAt(rotation: number): WireDocument {
  const engine: TokenEngine = {
    x: 0,
    y: 0,
    w: 100,
    h: 100,
    rotation,
    visual: null,
    actor_id: null,
    overrides: null,
    face: null,
  };
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

describe("TokenRotationControl", () => {
  it("renders nothing without a selected token", () => {
    render(TokenRotationControl, {
      context: setAppContextForTest({
        documents: storeWith(tokenAt(0)),
        dispatchIntent: vi.fn(),
        canEdit: () => true,
      }),
      props: { tokenId: null },
    });
    expect(screen.queryByLabelText("actors.tokenRotation")).toBeNull();
  });

  it("renders nothing when canEdit refuses", () => {
    render(TokenRotationControl, {
      context: setAppContextForTest({
        documents: storeWith(tokenAt(0)),
        dispatchIntent: vi.fn(),
        canEdit: () => false,
      }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByLabelText("actors.tokenRotation")).toBeNull();
  });

  it("shows the token's current raw rotation", () => {
    render(TokenRotationControl, {
      context: setAppContextForTest({
        documents: storeWith(tokenAt(45)),
        dispatchIntent: vi.fn(),
        canEdit: () => true,
      }),
      props: { tokenId: "tok1" },
    });
    expect(
      (screen.getByLabelText("actors.tokenRotation") as HTMLInputElement).value,
    ).toBe("45");
  });

  it("dispatches an /engine/rotation Update with the RAW stored value as the OCC pre-image", async () => {
    const dispatchIntent = vi.fn();
    render(TokenRotationControl, {
      context: setAppContextForTest({
        documents: storeWith(tokenAt(45)),
        dispatchIntent,
        canEdit: () => true,
      }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByLabelText("actors.tokenRotation"), {
      target: { value: "90" },
    });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [{ path: "/engine/rotation", old: 45, new: 90 }],
      },
    ]);
  });
});
