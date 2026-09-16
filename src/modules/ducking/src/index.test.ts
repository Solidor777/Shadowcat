import { describe, it, expect } from "vitest";
import { ducking } from "./index";

describe("ducking module scaffold", () => {
  it("has the expected manifest id", () => {
    expect(ducking.manifest.id).toBe("ducking");
  });
});
