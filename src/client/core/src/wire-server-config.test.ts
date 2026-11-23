// @vitest-environment node
import { describe, it, expect, expectTypeOf } from "vitest";
import type { z } from "zod";
import type * as Ts from "@shadowcat/types";
import {
  ServerConfigSchema,
  TlsStatusSchema,
  serverConfigSchemaImpl,
  tlsSourceSchemaImpl,
  tlsStatusSchemaImpl,
} from "./wire";

const TLS = {
  enabled: true,
  source: "self-signed" as const,
  https_origins: ["https://vtt.example"],
  alpn: ["h2", "http/1.1"],
};

describe("server config wire mirror — drift guard", () => {
  // The unannotated impl consts must infer EXACTLY the generated types: a field added on the
  // Rust side (for example `alpn`) that the schema lacks, or a narrowed member, fails typecheck.
  it("the schemas infer exactly the generated types", () => {
    expectTypeOf<z.infer<typeof tlsSourceSchemaImpl>>().toEqualTypeOf<Ts.TlsSource>();
    expectTypeOf<z.infer<typeof tlsStatusSchemaImpl>>().toEqualTypeOf<Ts.TlsStatus>();
    expectTypeOf<z.infer<typeof serverConfigSchemaImpl>>().toEqualTypeOf<Ts.ServerConfig>();
  });

  it("the tls block carries exactly the four published fields, alpn included", () => {
    expect(Object.keys(tlsStatusSchemaImpl.shape).sort()).toEqual([
      "alpn",
      "enabled",
      "https_origins",
      "source",
    ]);
  });
});

describe("ServerConfigSchema", () => {
  it("accepts the wire shape the server emits", () => {
    expect(
      ServerConfigSchema.parse({ initialized: true, tls: TLS, onboarding_enabled: true }),
    ).toEqual({
      initialized: true,
      tls: TLS,
      onboarding_enabled: true,
    });
    expect(
      ServerConfigSchema.parse({
        initialized: false,
        onboarding_enabled: false,
        tls: { enabled: false, source: null, https_origins: [], alpn: ["h2", "http/1.1"] },
      }).tls.source,
    ).toBeNull();
  });

  it("rejects a tls block missing alpn", () => {
    const { alpn: _dropped, ...withoutAlpn } = TLS;
    expect(() => TlsStatusSchema.parse(withoutAlpn)).toThrow();
    expect(() =>
      ServerConfigSchema.parse({ initialized: true, tls: withoutAlpn, onboarding_enabled: true }),
    ).toThrow();
  });

  it("rejects a body missing onboarding_enabled", () => {
    expect(() => ServerConfigSchema.parse({ initialized: true, tls: TLS })).toThrow();
  });

  it("rejects an unknown source and a missing tls block", () => {
    expect(() => TlsStatusSchema.parse({ ...TLS, source: "lets-encrypt" })).toThrow();
    expect(() => ServerConfigSchema.parse({ initialized: true })).toThrow();
  });
});
