// @vitest-environment node
import { describe, expect, it, vi } from "vitest";
import type { TlsStatus } from "@shadowcat/types";
import type { LocationLike } from "@shadowcat/core";
import { plainHttpHint, type PlainHttpHintDeps } from "./plainHttpHint";

const SELF_SIGNED: TlsStatus = { enabled: true, source: "self-signed", https_origins: [], alpn: ["h2", "http/1.1"] };

const PAGE: LocationLike = {
  protocol: "https:",
  hostname: "192.0.2.10",
  host: "192.0.2.10:30000",
  pathname: "/w",
  search: "",
  hash: "",
  origin: "https://192.0.2.10:30000",
};

function deps(over: Partial<PlainHttpHintDeps> = {}): PlainHttpHintDeps {
  return {
    location: PAGE,
    currentTls: () => SELF_SIGNED,
    loadTls: async () => SELF_SIGNED,
    fetchMe: async () => ({ ok: true }),
    ...over,
  };
}

describe("plainHttpHint", () => {
  it("offers the plain address when the probe answers while the socket keeps failing", async () => {
    expect(await plainHttpHint(3, deps())).toBe("http://192.0.2.10:30000/w");
  });

  it("stays silent unless the failure count is exactly the threshold", async () => {
    const fetchMe = vi.fn(async () => ({ ok: true }));
    expect(await plainHttpHint(2, deps({ fetchMe }))).toBeNull();
    expect(await plainHttpHint(4, deps({ fetchMe }))).toBeNull();
    expect(fetchMe).not.toHaveBeenCalled();
  });

  it("stays silent on an http page without probing", async () => {
    const fetchMe = vi.fn(async () => ({ ok: true }));
    expect(await plainHttpHint(3, deps({ location: { ...PAGE, protocol: "http:" }, fetchMe }))).toBeNull();
    expect(fetchMe).not.toHaveBeenCalled();
  });

  it("stays silent when the probe is unauthenticated (a bad session, not a refused wss)", async () => {
    expect(await plainHttpHint(3, deps({ fetchMe: async () => ({ ok: false }) }))).toBeNull();
  });

  it("stays silent when the probe fails outright (the server is down)", async () => {
    expect(
      await plainHttpHint(3, deps({ fetchMe: async () => Promise.reject(new Error("network")) })),
    ).toBeNull();
  });

  it("stays silent when the TLS status could not be read", async () => {
    const fetchMe = vi.fn(async () => ({ ok: true }));
    expect(await plainHttpHint(3, deps({ currentTls: () => null, loadTls: async () => null, fetchMe }))).toBeNull();
    expect(fetchMe).not.toHaveBeenCalled();
  });

  it("loads the status when none is cached", async () => {
    const loadTls = vi.fn(async () => SELF_SIGNED);
    expect(await plainHttpHint(3, deps({ currentTls: () => null, loadTls }))).toBe("http://192.0.2.10:30000/w");
    expect(loadTls).toHaveBeenCalledOnce();
  });

  it("stays silent for an operator certificate or a portless (proxy-served) page", async () => {
    expect(await plainHttpHint(3, deps({ currentTls: () => ({ ...SELF_SIGNED, source: "operator" }) }))).toBeNull();
    expect(
      await plainHttpHint(3, deps({ location: { ...PAGE, host: "192.0.2.10", origin: "https://192.0.2.10" } })),
    ).toBeNull();
  });
});
