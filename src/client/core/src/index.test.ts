import { describe, it, expect } from "vitest";
import { isHealthy } from "./index";

describe("isHealthy", () => {
  it("is true only when status is ok and the db is connected", () => {
    expect(isHealthy({ status: "ok", db_connected: true })).toBe(true);
    expect(isHealthy({ status: "ok", db_connected: false })).toBe(false);
    expect(isHealthy({ status: "down", db_connected: true })).toBe(false);
  });
});
