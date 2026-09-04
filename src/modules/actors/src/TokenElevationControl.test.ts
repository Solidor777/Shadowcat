import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildTokenDoc,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import TokenElevationControl from "./TokenElevationControl.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

function tokenWith(elevation: number | null): WireDocument {
  const engine: TokenEngine = {
    x: 0, y: 0, w: 100, h: 100, rotation: 0,
    visual: null, actor_id: null, overrides: null, face: null, elevation };
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

describe("TokenElevationControl", () => {
  it("renders nothing without a selected token or when canEdit refuses", () => {
    render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(null)), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: null },
    });
    expect(screen.queryByTestId("token-elevation")).toBeNull();

    render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(null)), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-elevation")).toBeNull();
  });

  it("renders for a RAW token too (elevation is token state, not an actor-linked override)", () => {
    render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(3)), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-elevation") as HTMLInputElement).value).toBe("3");
  });

  it("displays 0 at ground (absent stored value) and commits a finite elevation with the raw old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(null)), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const input = screen.getByTestId("token-elevation") as HTMLInputElement;
    expect(input.value).toBe("0");
    await fireEvent.change(input, { target: { value: "10" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/elevation", old: null, new: 10 }] },
    ]);
  });

  it("writing 0 normalizes to null (canonical grounded) with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(10)), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByTestId("token-elevation"), { target: { value: "0" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/elevation", old: 10, new: null }] },
    ]);
  });

  it("emptying the input writes null; a no-op value dispatches nothing", async () => {
    const dispatchIntent = vi.fn();
    const first = render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(10)), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(first.getByTestId("token-elevation"), { target: { value: "" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/elevation", old: 10, new: null }] },
    ]);
    first.unmount();

    // Grounded token, already canonically null: re-entering 0 must not dispatch.
    const dispatch2 = vi.fn();
    const second = render(TokenElevationControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(null)), dispatchIntent: dispatch2, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(second.getByTestId("token-elevation"), { target: { value: "0" } });
    expect(dispatch2).not.toHaveBeenCalled();
  });
});
