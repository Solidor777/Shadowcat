// @vitest-environment node
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { VISION_TREATMENT_CONTRACT } from "@shadowcat/core";
import type { Contribution, VisionTreatmentPaint } from "@shadowcat/core";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixture-modules", "vision-kind-fixture");

/** The fixture's entry module, loaded the way the browser loads it: by its file URL. */
const fixture = (await import(pathToFileURL(path.join(FIXTURE_DIR, "index.js")).href)) as {
  default: { register(ctx: { contributions: { contribute(c: Contribution): void } }): void };
  resolveGlow(params: unknown): VisionTreatmentPaint;
  TREATMENT_KIND: string;
  GLOW_COLOR: number;
};

describe("vision-kind-fixture module", () => {
  it("contributes one treatment under the vision-treatment contract, keyed by its kind", () => {
    const seen: Contribution[] = [];
    fixture.default.register({ contributions: { contribute: (c) => void seen.push(c) } });
    expect(seen).toHaveLength(1);
    expect(seen[0].contract).toBe(VISION_TREATMENT_CONTRACT);
    expect(seen[0].component).toBeNull();
    expect(seen[0].visionTreatment?.kind).toBe(fixture.TREATMENT_KIND);
    expect(seen[0].visionTreatment?.resolve({}, { corners: [] })).toEqual(fixture.resolveGlow({}));
  });

  it("resolves a fill by default and an outline when the params ask for one", () => {
    const glow = fixture.GLOW_COLOR;
    expect(fixture.resolveGlow(null)).toEqual({ shape: "fill", color: glow, opacity: 0.4 });
    expect(fixture.resolveGlow({ paint: "wash" })).toEqual({ shape: "fill", color: glow, opacity: 0.4 });
    expect(fixture.resolveGlow({ paint: "outline" })).toEqual({ shape: "outline", color: glow });
  });

  it("declares the folder-matching id and the any-version engine range the e2e enables it under", () => {
    const manifest = JSON.parse(readFileSync(path.join(FIXTURE_DIR, "module.json"), "utf8")) as {
      id: string;
      engines: { shadowcat: string };
    };
    expect(manifest.id).toBe("vision-kind-fixture");
    expect(manifest.engines.shadowcat).toBe("*");
  });
});
