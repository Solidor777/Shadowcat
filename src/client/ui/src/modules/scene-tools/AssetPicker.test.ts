import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { SceneInteractionBridge } from "../../lib/sceneInteraction";
import { setAppContextForTest } from "../../lib/__fixtures__/appContextTest";
import { ToolController } from "./controller.svelte";
import * as api from "../../lib/api";
import AssetPicker from "./AssetPicker.svelte";

beforeEach(() => vi.restoreAllMocks());

function makeController(): ToolController {
  return new ToolController({
    scene: new SceneInteractionBridge(),
    dispatchIntent: () => {},
    documents: new DocumentStore(),
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
  });
}

const asset = (id: string, content_type: string, name: string): unknown => ({
  id, world_id: "w1", storage_key: "", original_name: name, content_type, byte_size: 1, created_by: "u", created_at: 0, version: 1,
});

test("lists image assets and selecting one sets the controller's selectedAsset", async () => {
  vi.spyOn(api, "listAssets").mockResolvedValue([
    asset("img-1", "image/png", "goblin.png"),
    asset("doc-1", "application/pdf", "rules.pdf"), // non-image, filtered out
  ] as never);
  const controller = makeController();
  render(AssetPicker, { props: { controller }, context: setAppContextForTest({ role: "gm", world: "w1" }) });

  const tiles = await screen.findAllByTestId("picker-asset");
  expect(tiles).toHaveLength(1); // only the image asset is placeable
  await fireEvent.click(tiles[0]);
  expect(controller.selectedAsset).toBe("img-1");
  expect(tiles[0].getAttribute("aria-pressed")).toBe("true");
});
