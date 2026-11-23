// @vitest-environment node
import { describe, expect, it } from "vitest";
import { DEFAULT_E2E_PORT_BASE, E2E_PORT_BASE_ENV, resolveE2ePorts } from "./ports";

describe("resolveE2ePorts", () => {
  it("gives the default ports when the variable is unset or empty", () => {
    const defaults = { loopback: 31999, lan: 32000, https: 32001, mediaRange: 32002, longAudio: 32003, routePrefix: 32004 };
    expect(resolveE2ePorts({})).toEqual(defaults);
    expect(resolveE2ePorts({ [E2E_PORT_BASE_ENV]: "" })).toEqual(defaults);
    expect(DEFAULT_E2E_PORT_BASE).toBe(32000);
  });

  it("moves all six ports together, contiguous and distinct", () => {
    const ports = resolveE2ePorts({ [E2E_PORT_BASE_ENV]: "40000" });
    expect(ports).toEqual({ loopback: 39999, lan: 40000, https: 40001, mediaRange: 40002, longAudio: 40003, routePrefix: 40004 });
    expect(new Set(Object.values(ports)).size).toBe(6);
  });

  it("gives two different bases disjoint port sets", () => {
    const a = Object.values(resolveE2ePorts({ [E2E_PORT_BASE_ENV]: "32000" }));
    const b = Object.values(resolveE2ePorts({ [E2E_PORT_BASE_ENV]: "33000" }));
    expect(a.filter((p) => b.includes(p))).toEqual([]);
  });

  it("rejects a value that is not a whole number or whose ports leave the valid range", () => {
    for (const bad of ["abc", "32000.5", "1", "65532", "-5"]) {
      expect(() => resolveE2ePorts({ [E2E_PORT_BASE_ENV]: bad }), bad).toThrow(E2E_PORT_BASE_ENV);
    }
    expect(resolveE2ePorts({ [E2E_PORT_BASE_ENV]: "65531" }).routePrefix).toBe(65535);
  });
});
