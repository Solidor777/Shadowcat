// @vitest-environment node
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { RULER_LIMITS } from "./ruler";
import { randomUuid } from "./uuid";

describe("RULER_LIMITS", () => {
  // Shared conformance fixture, read on both sides: this file (via `readFileSync`, since the
  // package has no `resolveJsonModule`) and the server's `ws::conn::tests::ruler` limits test
  // (`include_str!`). One corpus, so the two sides' numbers cannot silently drift apart.
  const fixture: { maxPoints: number; maxOriginBytes: number; ratePerMin: number; maxCoord: number } = JSON.parse(
    readFileSync(fileURLToPath(new URL("./__fixtures__/ruler-limits.json", import.meta.url)), "utf-8"),
  );

  it("agrees with the server's limits on every field", () => {
    expect({ ...RULER_LIMITS }).toEqual(fixture);
  });

  it("leaves an honest origin token (one UUID) inside the server's origin bound", () => {
    expect(new TextEncoder().encode(randomUuid()).length).toBeLessThanOrEqual(RULER_LIMITS.maxOriginBytes);
  });
});
