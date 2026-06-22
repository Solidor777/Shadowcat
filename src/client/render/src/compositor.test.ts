import { test, expect } from "vitest";
import { Compositor, MockBackend } from "./index";

test("setVisibility forwards to the backend and is retrievable", () => {
  const backend = new MockBackend();
  const c = new Compositor(backend);
  c.setVisibility({ visible: [] }); // identity
  expect(backend.visibility).toEqual({ visible: [] });
  expect(c.current()).toEqual({ visible: [] });

  const poly = { visible: [{ points: [0, 0, 10, 0, 10, 10] }] };
  c.setVisibility(poly);
  expect(backend.visibility).toEqual(poly);
  expect(c.current()).toEqual(poly);
});
