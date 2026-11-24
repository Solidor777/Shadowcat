import { test, expect, describe } from "vitest";
import { buildRegionFromTemplate, buildSceneEntityDoc, templateToRegionShape, type RegionEngine, type TemplateEngine, type TemplateShape } from "./scene-docs";
import { conePoints, linePoints, squarePoints } from "./geometry";

const shape = (over: Partial<TemplateShape> & Pick<TemplateShape, "kind">): TemplateShape => ({
  x: 300, y: 400, size: 100, direction: 30, angle: null, width: null, ...over,
});

const templateDoc = (s: TemplateShape, elevation: TemplateEngine["elevation"] = null) =>
  buildSceneEntityDoc("w1", "s1", "template", { shape: s, color: "#3388ff", elevation } satisfies TemplateEngine, "t1");

describe("templateToRegionShape", () => {
  test("a circle is an exact region circle [x, y, size], not a polygon approximation", () => {
    expect(templateToRegionShape(shape({ kind: "circle" }))).toEqual({ ok: true, shape: { kind: "circle", points: [300, 400, 100] } });
  });

  test("a cone is the polygon conePoints draws, using the authored aperture", () => {
    const r = templateToRegionShape(shape({ kind: "cone", angle: 90 }));
    expect(r).toEqual({ ok: true, shape: { kind: "polygon", points: conePoints(300, 400, 100, 30, 90) } });
  });

  test("a cone without an authored angle uses the default aperture", () => {
    const r = templateToRegionShape(shape({ kind: "cone" }));
    expect(r).toEqual({ ok: true, shape: { kind: "polygon", points: conePoints(300, 400, 100, 30) } });
  });

  test("a rect with no width is the rotated square; a distinct width is the second axis", () => {
    expect(templateToRegionShape(shape({ kind: "rect" }))).toEqual({ ok: true, shape: { kind: "polygon", points: squarePoints(300, 400, 100, 30, 100) } });
    expect(templateToRegionShape(shape({ kind: "rect", width: 40 }))).toEqual({ ok: true, shape: { kind: "polygon", points: squarePoints(300, 400, 100, 30, 40) } });
  });

  test("a line with a positive width is the thick-line polygon", () => {
    const r = templateToRegionShape(shape({ kind: "line", width: 20 }));
    expect(r).toEqual({ ok: true, shape: { kind: "polygon", points: linePoints(300, 400, 100, 30, 20) } });
    if (r.ok) expect(r.shape.points).toHaveLength(8);
  });

  test("a line with an absent or zero width is refused with a reason", () => {
    expect(templateToRegionShape(shape({ kind: "line" }))).toEqual({ ok: false, reason: "zero_width_line" });
    expect(templateToRegionShape(shape({ kind: "line", width: 0 }))).toEqual({ ok: false, reason: "zero_width_line" });
  });

  test("a cone whose angle the render layer refuses to draw is refused", () => {
    for (const angle of [0, 180, 200, -10, Number.NaN]) {
      expect(templateToRegionShape(shape({ kind: "cone", angle }))).toEqual({ ok: false, reason: "unknown_shape" });
    }
  });

  test("an unrecognised kind is refused", () => {
    expect(templateToRegionShape(shape({ kind: "blob" }))).toEqual({ ok: false, reason: "unknown_shape" });
  });
});

describe("buildRegionFromTemplate", () => {
  test("is one batch: create the region, delete the template with its full pre-image", () => {
    const tpl = templateDoc(shape({ kind: "circle" }));
    const r = buildRegionFromTemplate("w1", tpl, "r1");
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.ops).toEqual([
      { op: "create", doc: r.region },
      { op: "delete", doc: tpl },
    ]);
  });

  test("the region is a plain terrain region in the template's scene, with the world-default visibility", () => {
    const tpl = templateDoc(shape({ kind: "cone" }));
    const r = buildRegionFromTemplate("w1", tpl);
    if (!r.ok) throw new Error("expected a conversion");
    expect(r.region.doc_type).toBe("region");
    expect(r.region.parent_id).toBe("s1");
    expect(r.region.permissions.default).toBe("observer");
    const eng = r.region.engine as RegionEngine;
    expect(eng).toMatchObject({ behavior: "terrain", cost: 1, cost_by_tag: {}, enabled: true, triggers: [], ambient_override: null });
  });

  test("the elevation band is copied; the colour is not carried", () => {
    const band = { bottom: 2, top: 5 };
    const r = buildRegionFromTemplate("w1", templateDoc(shape({ kind: "rect" }), band));
    if (!r.ok) throw new Error("expected a conversion");
    const eng = r.region.engine as RegionEngine;
    expect(eng.elevation).toEqual(band);
    expect(JSON.stringify(eng)).not.toContain("3388ff");
  });

  test("a zero-width line yields the refusal and no operations", () => {
    expect(buildRegionFromTemplate("w1", templateDoc(shape({ kind: "line" })))).toEqual({ ok: false, reason: "zero_width_line" });
  });
});
