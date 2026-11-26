// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { randomUuid } from "./uuid";

const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("randomUuid", () => {
  it("uses crypto.randomUUID when the platform provides it", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "11111111-2222-4333-8444-555555555555" });
    expect(randomUuid()).toBe("11111111-2222-4333-8444-555555555555");
  });

  it("builds a well-formed, distinct v4 UUID from getRandomValues when randomUUID is absent", () => {
    const real = globalThis.crypto;
    vi.stubGlobal("crypto", { getRandomValues: real.getRandomValues.bind(real) });
    expect(typeof crypto.randomUUID).toBe("undefined");
    const ids = Array.from({ length: 200 }, () => randomUuid());
    for (const id of ids) expect(id).toMatch(V4);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("sets the version and variant bits over any random input (all-ones and all-zero bytes)", () => {
    for (const fill of [0x00, 0xff]) {
      vi.stubGlobal("crypto", {
        getRandomValues: (a: Uint8Array) => {
          a.fill(fill);
          return a;
        },
      });
      expect(randomUuid()).toMatch(V4);
    }
    // 0x00 bytes: version nibble 4, variant 8; 0xff bytes: version nibble 4, variant b.
    vi.stubGlobal("crypto", { getRandomValues: (a: Uint8Array) => a.fill(0) });
    expect(randomUuid()).toBe("00000000-0000-4000-8000-000000000000");
    vi.stubGlobal("crypto", { getRandomValues: (a: Uint8Array) => a.fill(0xff) });
    expect(randomUuid()).toBe("ffffffff-ffff-4fff-bfff-ffffffffffff");
  });
});
