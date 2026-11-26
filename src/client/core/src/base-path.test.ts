import { afterEach, describe, expect, it } from "vitest";
import { apiPath, basePath, wsUrl } from "./base-path";

/** The global the server's `index.html` substitution writes. */
const BASE_KEY = "__SHADOWCAT_BASE__";

/** Sets or clears the injected route-prefix global. */
function setBase(value: string | undefined): void {
  if (value === undefined) {
    delete (globalThis as Record<string, unknown>)[BASE_KEY];
  } else {
    (globalThis as Record<string, unknown>)[BASE_KEY] = value;
  }
}

describe("base-path", () => {
  afterEach(() => setBase(undefined));

  it("is the identity at the root mount (global absent or empty)", () => {
    expect(basePath()).toBe("");
    expect(apiPath("/api/config")).toBe("/api/config");
    setBase("");
    expect(apiPath("/api/config")).toBe("/api/config");
    expect(wsUrl({ protocol: "http:", host: "lan.example:30000" })).toBe(
      "ws://lan.example:30000/ws",
    );
  });

  it("prepends the injected prefix to every path", () => {
    setBase("/ttrpg");
    expect(basePath()).toBe("/ttrpg");
    expect(apiPath("/api/worlds/w1/assets?kind=image")).toBe(
      "/ttrpg/api/worlds/w1/assets?kind=image",
    );
    expect(apiPath("/modules/dnd5e/index.js")).toBe("/ttrpg/modules/dnd5e/index.js");
  });

  it("builds a wss URL for https pages and a ws URL otherwise, under the prefix", () => {
    setBase("/ttrpg");
    expect(wsUrl({ protocol: "https:", host: "play.example" })).toBe(
      "wss://play.example/ttrpg/ws",
    );
    expect(wsUrl({ protocol: "http:", host: "lan.example:30000" })).toBe(
      "ws://lan.example:30000/ttrpg/ws",
    );
  });

  it("reads the page location when no origin is passed", () => {
    setBase("/ttrpg");
    const saved = Object.getOwnPropertyDescriptor(globalThis, "location");
    Object.defineProperty(globalThis, "location", {
      value: { protocol: "https:", host: "page.example" },
      configurable: true,
    });
    try {
      expect(wsUrl()).toBe("wss://page.example/ttrpg/ws");
    } finally {
      if (saved) Object.defineProperty(globalThis, "location", saved);
      else delete (globalThis as Record<string, unknown>).location;
    }
  });
});
