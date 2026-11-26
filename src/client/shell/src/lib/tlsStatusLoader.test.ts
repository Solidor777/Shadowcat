// @vitest-environment node
import { describe, expect, it, vi } from "vitest";
import type { TlsStatus } from "@shadowcat/types";
import { TlsStatusLoader } from "./tlsStatusLoader.svelte";

const STATUS: TlsStatus = { enabled: true, source: "self-signed", https_origins: [], alpn: ["h2"] };
const noSleep = async () => {};

describe("TlsStatusLoader", () => {
  it("reads once and then serves the cached status", async () => {
    const load = vi.fn(async () => STATUS);
    const loader = new TlsStatusLoader(load, [1], noSleep);
    expect(await loader.ensure()).toBe(STATUS);
    expect(await loader.ensure()).toBe(STATUS);
    expect(load).toHaveBeenCalledOnce();
    expect(loader.status).toBe(STATUS);
  });

  it("retries a failed read after each configured delay", async () => {
    const sleeps: number[] = [];
    const load = vi.fn().mockRejectedValueOnce(new Error("a")).mockRejectedValueOnce(new Error("b")).mockResolvedValue(STATUS);
    const loader = new TlsStatusLoader(load, [10, 20, 40], async (ms) => void sleeps.push(ms));
    expect(await loader.ensure()).toBe(STATUS);
    expect(load).toHaveBeenCalledTimes(3);
    expect(sleeps).toEqual([10, 20]);
  });

  it("gives up after the last delay, leaving the status null", async () => {
    const load = vi.fn(async () => Promise.reject(new Error("down")));
    const loader = new TlsStatusLoader(load, [1, 1], noSleep);
    expect(await loader.ensure()).toBeNull();
    expect(load).toHaveBeenCalledTimes(3);
    expect(loader.status).toBeNull();
  });

  it("reads again on the next ensure() after giving up (a later welcome or hint)", async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue(STATUS);
    const loader = new TlsStatusLoader(load, [], noSleep);
    expect(await loader.ensure()).toBeNull();
    expect(await loader.ensure()).toBe(STATUS);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("shares one request loop between concurrent callers", async () => {
    let resolve!: (s: TlsStatus) => void;
    const load = vi.fn(() => new Promise<TlsStatus>((r) => (resolve = r)));
    const loader = new TlsStatusLoader(load, [], noSleep);
    const a = loader.ensure();
    const b = loader.ensure();
    resolve(STATUS);
    expect(await Promise.all([a, b])).toEqual([STATUS, STATUS]);
    expect(load).toHaveBeenCalledOnce();
  });
});
