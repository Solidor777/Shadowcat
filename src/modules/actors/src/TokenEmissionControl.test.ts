import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildTokenDoc, type TokenEngine, type WireDocument, type WireOperation } from "@shadowcat/core";
import TokenEmissionControl from "./TokenEmissionControl.svelte";

// Suppress listAssets fetch: EmissionEditor calls listAssets($effect) which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

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
 * token — the control hides for instanced/raw tokens). */
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

function tokenWith(engine: TokenEngine): WireDocument {
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

describe("TokenEmissionControl", () => {
  it("renders nothing without a selected token", () => {
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(tokenEngine(true))), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: null },
    });
    expect(screen.queryByLabelText("actors.aura")).toBeNull();
  });

  it("renders nothing for an instanced (unlinked) token — its overrides would be inert", () => {
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(tokenEngine(false))), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByLabelText("actors.aura")).toBeNull();
  });

  it("renders nothing when canEdit refuses", () => {
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tokenWith(tokenEngine(true))), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByLabelText("actors.aura")).toBeNull();
  });

  it("reflects the token's raw stored overrides", () => {
    const tok = tokenWith(
      tokenEngine(true, { name: null, visual: null, size: null, shape: null, vision: null, aura: { color: "#0000ff", opacity: 0.9, radius: 3, enabled: true }, sound: null, vfx: null , light: null, movement: null }),
    );
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tok), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByLabelText("actors.auraRadius") as HTMLInputElement).value).toBe("3");
    expect(screen.queryByLabelText("actors.soundRadius")).toBeNull(); // sound override absent
  });

  it("dispatches an /engine/overrides/aura Update with the RAW stored value as the OCC pre-image", async () => {
    const dispatchIntent = vi.fn();
    const existing = { color: "#0000ff", opacity: 0.9, radius: 3, enabled: true };
    const tok = tokenWith(
      tokenEngine(true, { name: null, visual: null, size: null, shape: null, vision: null, aura: existing, sound: null, vfx: null , light: null, movement: null }),
    );
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tok), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByLabelText("actors.auraRadius"), { target: { value: "5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [{ path: "/engine/overrides/aura", old: existing, new: { ...existing, radius: 5 } }],
      },
    ]);
  });

  it("dispatches old:null when no override exists yet, and clearing toggles back to null", async () => {
    const dispatchIntent = vi.fn();
    const tok = tokenWith(tokenEngine(true));
    render(TokenEmissionControl, {
      context: setAppContextForTest({ documents: storeWith(tok), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    // Toggle the vfx section on: writes the default payload with a null pre-image.
    await fireEvent.click(screen.getByLabelText("actors.vfx"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [{ path: "/engine/overrides/vfx", old: null, new: { asset: "", anchor: "token", loop: true, enabled: true } }],
      },
    ]);
  });
});
