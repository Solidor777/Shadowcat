// @vitest-environment node
import { describe, expect, it } from "vitest";
import type { TlsStatus } from "@shadowcat/types";
import {
  httpWorksWhileSocketFails,
  DISMISS_KEY_PREFIX,
  dismiss,
  httpsTargets,
  isDismissed,
  isLoopbackHost,
  plainHttpUrl,
  shouldHintPlainHttp,
  shouldShowHttpsNotice,
  type LocationLike,
} from "./https-access";

const TLS: TlsStatus = {
  enabled: true,
  source: "self-signed",
  https_origins: [],
  alpn: ["h2", "http/1.1"],
};

function loc(over: Partial<LocationLike> = {}): LocationLike {
  return {
    protocol: "http:",
    hostname: "192.168.1.20",
    host: "192.168.1.20:30000",
    pathname: "/",
    search: "",
    hash: "",
    origin: "http://192.168.1.20:30000",
    ...over,
  };
}

describe("shouldShowHttpsNotice", () => {
  it("shows on plain http at a LAN address when TLS is enabled", () => {
    expect(shouldShowHttpsNotice(TLS, loc())).toBe(true);
  });
  it("hides on https, on loopback, and when TLS is off", () => {
    expect(shouldShowHttpsNotice(TLS, loc({ protocol: "https:" }))).toBe(false);
    expect(shouldShowHttpsNotice(TLS, loc({ hostname: "localhost" }))).toBe(false);
    expect(shouldShowHttpsNotice(TLS, loc({ hostname: "127.0.0.1" }))).toBe(false);
    expect(shouldShowHttpsNotice({ ...TLS, enabled: false }, loc())).toBe(false);
  });
});

describe("isLoopbackHost", () => {
  it("recognises the loopback spellings and nothing else", () => {
    for (const h of ["localhost", "LOCALHOST", "127.0.0.1", "127.9.9.9", "[::1]", "::1"]) {
      expect(isLoopbackHost(h)).toBe(true);
    }
    for (const h of ["192.168.1.20", "10.0.0.1", "vtt.example", "127.0.0.1.example"]) {
      expect(isLoopbackHost(h)).toBe(false);
    }
  });
});

describe("httpsTargets", () => {
  it("swaps the scheme of the address that worked when nothing is published", () => {
    expect(httpsTargets(TLS, loc({ search: "?a=1", hash: "#x" }))).toEqual([
      "https://192.168.1.20:30000/?a=1#x",
    ]);
  });
  it("lists two spellings of one published origin once, so a keyed list never sees a duplicate", () => {
    const tls = {
      ...TLS,
      https_origins: ["https://Game.Example:8443", "https://game.example:8443/", "https://game.example:8443"],
    };
    expect(httpsTargets(tls, loc())).toEqual(["https://game.example:8443/"]);
  });
  it("prefers the published origin naming the host in use", () => {
    const tls = {
      ...TLS,
      https_origins: ["https://other.example:8443", "https://192.168.1.20:30000"],
    };
    expect(httpsTargets(tls, loc())).toEqual(["https://192.168.1.20:30000/"]);
  });
  it("offers every published origin when none names the host in use", () => {
    const tls = { ...TLS, https_origins: ["https://a.example/", "https://b.example:8443"] };
    expect(httpsTargets(tls, loc())).toEqual(["https://a.example/", "https://b.example:8443/"]);
  });
  it("ignores an unparseable or non-https published origin and falls back to the swap", () => {
    const tls = { ...TLS, https_origins: ["not a url", "http://plain.example"] };
    expect(httpsTargets(tls, loc())).toEqual(["https://192.168.1.20:30000/"]);
  });
  it("keeps a valid origin beside an unparseable one", () => {
    const tls = { ...TLS, https_origins: ["not a url", "https://a.example"] };
    expect(httpsTargets(tls, loc())).toEqual(["https://a.example/"]);
  });
  it("offers no implicit-port swap when nothing is published", () => {
    const portless = { ...loc(), host: "192.168.1.20" };
    expect(httpsTargets({ ...TLS, https_origins: [] }, portless)).toEqual([]);
    expect(shouldShowHttpsNotice({ ...TLS, https_origins: [] }, portless)).toBe(false);
  });
});

describe("httpWorksWhileSocketFails", () => {
  it("is true only when the same-origin probe succeeds", async () => {
    expect(await httpWorksWhileSocketFails(async () => ({ ok: true }))).toBe(true);
  });
  it("is false for a 401 or 5xx (the session or server is the problem)", async () => {
    expect(await httpWorksWhileSocketFails(async () => ({ ok: false }))).toBe(false);
  });
  it("is false when the server is unreachable", async () => {
    expect(
      await httpWorksWhileSocketFails(async () => {
        throw new TypeError("Failed to fetch");
      }),
    ).toBe(false);
  });
});

describe("dismissal", () => {
  it("round-trips per origin", () => {
    const map = new Map<string, string>();
    const storage = {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
    };
    expect(isDismissed(storage, "http://a")).toBe(false);
    dismiss(storage, "http://a");
    expect(map.get(DISMISS_KEY_PREFIX + "http://a")).toBe("1");
    expect(isDismissed(storage, "http://a")).toBe(true);
    expect(isDismissed(storage, "http://b")).toBe(false);
  });
  it("never throws when storage is absent or throws", () => {
    const boom = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(isDismissed(null, "o")).toBe(false);
    expect(isDismissed(boom, "o")).toBe(false);
    expect(() => dismiss(null, "o")).not.toThrow();
    expect(() => dismiss(boom, "o")).not.toThrow();
  });
});

describe("shouldHintPlainHttp", () => {
  it("fires only at the threshold on an https page with a self-signed certificate", () => {
    const page = { protocol: "https:", host: "192.168.1.20:30000" };
    expect(shouldHintPlainHttp(2, page, TLS)).toBe(false);
    expect(shouldHintPlainHttp(3, page, TLS)).toBe(true);
    expect(shouldHintPlainHttp(4, page, TLS)).toBe(false);
  });
  it("stays silent on http pages, other certificate sources and an unread status", () => {
    const page = { protocol: "https:", host: "192.168.1.20:30000" };
    expect(shouldHintPlainHttp(3, { ...page, protocol: "http:" }, TLS)).toBe(false);
    expect(shouldHintPlainHttp(3, page, { ...TLS, source: "operator" })).toBe(false);
    expect(shouldHintPlainHttp(3, page, null)).toBe(false);
  });
  it("stays silent on a portless page, which a TLS-terminating proxy serves", () => {
    expect(shouldHintPlainHttp(3, { protocol: "https:", host: "vtt.example" }, TLS)).toBe(false);
  });
});

describe("plainHttpUrl", () => {
  it("keeps host, port, path, query and fragment", () => {
    expect(
      plainHttpUrl(loc({ protocol: "https:", pathname: "/a", search: "?b=1", hash: "#c" })),
    ).toBe("http://192.168.1.20:30000/a?b=1#c");
  });
});
