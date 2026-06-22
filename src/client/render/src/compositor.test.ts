import { test, expect } from "vitest";
import { Compositor, MockBackend } from "./index";

test("setVisibility forwards to the backend and is retrievable", () => {
  const backend = new MockBackend();
  const c = new Compositor(backend);
  c.setVisibility({ mode: "all", visible: [] }); // GM / no fog
  expect(backend.visibility).toEqual({ mode: "all", visible: [] });
  expect(c.current()).toEqual({ mode: "all", visible: [] });

  const poly = { mode: "masked" as const, visible: [{ points: [0, 0, 10, 0, 10, 10] }] };
  c.setVisibility(poly);
  expect(backend.visibility).toEqual(poly);
  expect(c.current()).toEqual(poly);
});
