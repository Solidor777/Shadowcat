import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildTokenDoc,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import TokenRotationLockControl from "./TokenRotationLockControl.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

function tokenWith(lock_rotation: boolean, actor_id: string | null = null): WireDocument {
  const engine: TokenEngine = {
    x: 0, y: 0, w: 100, h: 100, rotation: 0,
    visual: null, actor_id, overrides: null, face: null, elevation: null, lock_rotation };
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

describe("TokenRotationLockControl", () => {
  it("renders nothing without a selected token or when canEdit refuses", () => {
    render(TokenRotationLockControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(false)), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: null },
    });
    expect(screen.queryByTestId("token-rotation-lock")).toBeNull();

    render(TokenRotationLockControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(false)), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-rotation-lock")).toBeNull();
  });

  it("shows the stored state for a raw token and a linked token alike", () => {
    render(TokenRotationLockControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(true, "actor1")), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-rotation-lock") as HTMLInputElement).checked).toBe(true);
  });

  it("checking writes /engine/lock_rotation with the stored old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenRotationLockControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(false)), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.click(screen.getByTestId("token-rotation-lock"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/lock_rotation", old: false, new: true }] },
    ]);
  });

  it("unchecking writes false over the stored true", async () => {
    const dispatchIntent = vi.fn();
    render(TokenRotationLockControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(true)), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.click(screen.getByTestId("token-rotation-lock"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/lock_rotation", old: true, new: false }] },
    ]);
  });
});
