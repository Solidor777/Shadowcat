// @vitest-environment node
import { describe, expect, it } from "vitest";
import type { Asset } from "@shadowcat/types";
import { AssetResolver } from "./assets";
import { resolveVfxSource } from "./vfx";

function asset(id: string, tags: string[] = []): Asset {
  return {
    id,
    world_id: "w1",
    original_name: `${id}.webp`,
    content_type: "image/webp",
    byte_size: 1n,
    created_by: null,
    created_at: 0n,
    storage_key: `w1/${id}`,
    version: 1n,
    folder_id: null,
    tags,
    derived_tags: [],
    width: null,
    height: null,
    has_alpha: false,
    animated: true,
    original_content_type: "image/webp",
    original_byte_size: 1n,
    original_retained: false,
    conversion_note: null,
    sheet: null,
  };
}

describe("resolveVfxSource", () => {
  it("resolves the server-derived grid sheet when meta.sheet is present", () => {
    const a = asset("a1");
    a.sheet = { rows: 2, cols: 2, count: 3, frame_ms: [100, 200, 300], width: 8, height: 8 };
    const src = resolveVfxSource(a, new AssetResolver());
    expect(src).toEqual({
      type: "sheet",
      url: "/api/assets/a1?variant=sheet",
      rows: 2,
      cols: 2,
      count: 3,
      frameMs: [100, 200, 300],
    });
  });

  it("resolves a PixiJS spritesheet pairing from a vfx:sheet= tag", () => {
    const a = asset("img1", ["vfx", "vfx:sheet=json1"]);
    const src = resolveVfxSource(a, new AssetResolver());
    expect(src).toEqual({
      kind: "sheet",
      imageUrl: "/api/assets/img1",
      sheetUrl: "/api/assets/json1",
      animation: "default",
    });
  });

  it("fails closed to null when neither a sheet nor a pairing tag exists", () => {
    expect(resolveVfxSource(asset("plain"), new AssetResolver())).toBeNull();
  });

  it("fails closed on a garbled vfx:sheet= tag with an empty id", () => {
    const a = asset("img2", ["vfx:sheet="]);
    expect(resolveVfxSource(a, new AssetResolver())).toBeNull();
  });

  it("prefers the server-derived sheet when both resolution paths exist", () => {
    const a = asset("both", ["vfx:sheet=json9"]);
    a.sheet = { rows: 1, cols: 2, count: 2, frame_ms: [50, 50], width: 4, height: 4 };
    const src = resolveVfxSource(a, new AssetResolver());
    expect(src).toEqual({
      type: "sheet",
      url: "/api/assets/both?variant=sheet",
      rows: 1,
      cols: 2,
      count: 2,
      frameMs: [50, 50],
    });
  });
});
