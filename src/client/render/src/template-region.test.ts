import { test, expect } from "vitest";
import { buildRegionFromTemplate, buildSceneEntityDoc, type TemplateEngine, type TemplateShape, type WireDocument } from "@shadowcat/core";
import { templateShapeSpec } from "./template-view";
import { regionShapeSpec } from "./region-view";

const templateDoc = (shape: TemplateShape): WireDocument =>
  buildSceneEntityDoc("w1", "s1", "template", { shape, color: "#3388ff", elevation: null } satisfies TemplateEngine, "t1");

// The converted region must cover what the template drew. For polygon kinds the region outline
// is point-for-point the template outline; for a circle the region circle's centre and radius
// are the ones every template outline vertex lies on.
test.each([
  { name: "cone", shape: { kind: "cone", x: 120, y: 80, size: 200, direction: 45, angle: 70, width: null } },
  { name: "rect", shape: { kind: "rect", x: 120, y: 80, size: 90, direction: 20, angle: null, width: 40 } },
  { name: "square rect", shape: { kind: "rect", x: 120, y: 80, size: 90, direction: 20, angle: null, width: null } },
  { name: "thick line", shape: { kind: "line", x: 120, y: 80, size: 300, direction: 135, angle: null, width: 30 } },
] as { name: string; shape: TemplateShape }[])("a converted $name region draws the same outline as the template", ({ shape }) => {
  const tpl = templateDoc(shape);
  const batch = buildRegionFromTemplate("w1", tpl);
  if (!batch.ok) throw new Error("expected a conversion");
  const drawn = templateShapeSpec(tpl);
  const regionSpec = regionShapeSpec(batch.region);
  expect(drawn).not.toBeNull();
  expect(regionSpec?.points).toEqual(drawn?.points);
});

test("a converted circle region is the circle every template outline vertex lies on", () => {
  const tpl = templateDoc({ kind: "circle", x: 120, y: 80, size: 75, direction: 0, angle: null, width: null });
  const batch = buildRegionFromTemplate("w1", tpl);
  if (!batch.ok) throw new Error("expected a conversion");
  const eng = batch.region.engine as { shape: { kind: string; points: number[] } };
  expect(eng.shape).toEqual({ kind: "circle", points: [120, 80, 75] });
  const pts = templateShapeSpec(tpl)!.points;
  for (let i = 0; i < pts.length; i += 2) expect(Math.hypot(pts[i] - 120, pts[i + 1] - 80)).toBeCloseTo(75);
});
