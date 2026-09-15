// @vitest-environment node
import { describe, it, expect, vi, afterEach } from "vitest";
import { AssetResolver } from "@shadowcat/core";
import type { AudioChannelId } from "@shadowcat/core";
import type { GainNodeLike } from "./context";
import { decodeAudioCandidate, decodeCandidates } from "./decode";
import { OneShotPlayer, ONE_SHOT_CACHE_BUDGET_BYTES } from "./one-shot-player";
import { oggBytes, stubAudioContext, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

afterEach(() => vi.restoreAllMocks());

function channelGains(): Record<AudioChannelId, GainNodeLike> {
  const gain = (): GainNodeLike => ({
    gain: { value: 1, setTargetAtTime: () => {} },
    connect: () => {},
    disconnect: () => {},
  });
  return { master: gain(), music: gain(), ambience: gain(), sfx: gain(), ui: gain() };
}

function mockFetchBytes(bytes: ArrayBuffer) {
  return vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(bytes));
}

const URLS = {
  ogg: "/api/assets/a1?variant=opus",
  webm: "/api/assets/a1?variant=opus-webm",
  fallback: "/api/assets/a1",
  oggType: "audio/ogg; codecs=opus",
  webmType: "audio/webm; codecs=opus",
};

describe("decodeCandidates", () => {
  it("a loop prefers the ogg derivative; a one-shot prefers the native original", () => {
    expect(decodeCandidates(URLS, "loop")).toEqual([URLS.ogg, URLS.webm, URLS.fallback]);
    expect(decodeCandidates(URLS, "oneshot")).toEqual([URLS.fallback, URLS.webm, URLS.ogg]);
  });
});

describe("decodeAudioCandidate", () => {
  it("decodes natively when the browser can", async () => {
    mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    const buffer = await decodeAudioCandidate(ctx, URLS, "oneshot", async () => stubWasmDecoder());
    expect(buffer.duration).toBe(1);
  });

  it("advances past a 404 candidate to the next one", async () => {
    const f = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(new Response(null, { status: 404 }))
      .mockResolvedValueOnce(new Response(wavBytes()));
    const ctx = stubAudioContext();
    await decodeAudioCandidate(ctx, URLS, "loop", async () => stubWasmDecoder());
    expect(f.mock.calls.map((c) => c[0])).toEqual([URLS.ogg, URLS.webm]);
  });

  it("falls back to the WASM decoder when native decode fails on Ogg bytes", async () => {
    mockFetchBytes(oggBytes());
    const ctx = stubAudioContext();
    ctx.decodeAudioData = async () => {
      throw new Error("native decode failed");
    };
    const decodeFile = vi.fn().mockResolvedValue({
      channelData: [new Float32Array(48_000)],
      sampleRate: 48_000,
    });
    const buffer = await decodeAudioCandidate(ctx, URLS, "loop", async () => ({
      decodeFile,
      free: () => {},
    }));
    expect(decodeFile).toHaveBeenCalledTimes(1);
    expect(buffer.duration).toBe(1);
  });

  it("does NOT invoke the WASM decoder for a non-Ogg native failure — it advances", async () => {
    mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    ctx.decodeAudioData = async () => {
      throw new Error("native decode failed");
    };
    const decodeFile = vi.fn();
    await expect(
      decodeAudioCandidate(ctx, URLS, "loop", async () => ({ decodeFile, free: () => {} })),
    ).rejects.toThrow("native decode failed");
    expect(decodeFile).not.toHaveBeenCalled();
  });
});

describe("OneShotPlayer", () => {
  it("concurrent misses share ONE in-flight decode (and one fetch)", async () => {
    const f = mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    const player = new OneShotPlayer(ctx, new AssetResolver(), channelGains(), async () => stubWasmDecoder());
    const [a, b] = await Promise.all([player.getBuffer("a1"), player.getBuffer("a1")]);
    expect(a).toBe(b);
    expect(f).toHaveBeenCalledTimes(1);
  });

  it("decodes and caches on first play; a second play reuses the buffer (fetch once)", async () => {
    const f = mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    const player = new OneShotPlayer(ctx, new AssetResolver(), channelGains(), async () => stubWasmDecoder());
    await player.play("a1");
    await player.play("a1");
    expect(f).toHaveBeenCalledTimes(1);
  });

  it("a one-shot fetches the native original FIRST", async () => {
    const f = mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    const player = new OneShotPlayer(ctx, new AssetResolver(), channelGains(), async () => stubWasmDecoder());
    await player.play("a1");
    expect(f.mock.calls[0][0]).toBe("/api/assets/a1");
  });

  it("evicts the OLDEST entry (LRU) when the budget is exceeded, never the newest", async () => {
    mockFetchBytes(wavBytes());
    const ctx = stubAudioContext();
    const player = new OneShotPlayer(ctx, new AssetResolver(), channelGains(), async () => stubWasmDecoder());
    // Stub buffers estimate at 1s × 48k × 4B = 192,000 bytes each.
    const perBuffer = 48_000 * 4;
    const count = Math.ceil(ONE_SHOT_CACHE_BUDGET_BYTES / perBuffer) + 1;
    for (let i = 0; i < count; i++) {
      await player.getBuffer(`asset-${i}`);
    }
    // The oldest (asset-0) was evicted; the newest survives. A getBuffer on
    // asset-0 re-fetches (cache miss), the newest is a hit.
    const f = vi.spyOn(globalThis, "fetch");
    const callsBefore = f.mock.calls.length;
    await player.getBuffer(`asset-${count - 1}`);
    expect(f.mock.calls.length).toBe(callsBefore);
    await player.getBuffer("asset-0");
    expect(f.mock.calls.length).toBe(callsBefore + 1);
  });
});
