import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import VisualKindEditor from "./VisualKindEditor.svelte";

// Suppress listAssets fetch: the editor calls listAssets in an $effect which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

describe("VisualKindEditor", () => {
  it("emits a complete image visual via onBuild when an asset is picked (image kind)", async () => {
    const onBuild = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));

    expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1" });
  });

  it("emits null via onBuild for an incomplete faces row (no asset picked)", async () => {
    const onBuild = vi.fn();
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    await fireEvent.input(screen.getByLabelText("actors.faceName"), { target: { value: "normal" } });
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });

    // An image face row with no asset picked is incomplete → the whole faces visual is null.
    expect(onBuild).toHaveBeenLastCalledWith(null);
  });
});
