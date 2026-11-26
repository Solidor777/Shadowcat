// @vitest-environment node
import { describe, expect, it } from "vitest";
import { judgeAddress } from "./lan-address";
import { judgeOutcome } from "./outcome-verdict";

const TABLE = { "project-a": { win32: "blocked", linux: "frame" } } as const;

describe("judgeOutcome", () => {
  it("matches the value measured for this project on this platform", () => {
    expect(judgeOutcome(TABLE, "project-a", "win32", "blocked", false)).toEqual({ kind: "match" });
    expect(judgeOutcome(TABLE, "project-a", "linux", "frame", true)).toEqual({ kind: "match" });
  });

  it("reports a differing observation as a mismatch naming both values", () => {
    const v = judgeOutcome(TABLE, "project-a", "win32", "frame", false);
    expect(v.kind).toBe("mismatch");
    expect(v).toMatchObject({ expected: "blocked" });
    expect((v as { message: string }).message).toContain('"frame"');
  });

  it("keys on the platform: a value measured on one OS does not cover another", () => {
    expect(judgeOutcome(TABLE, "project-a", "darwin", "blocked", false).kind).toBe("unmeasured-annotate");
  });

  it("keys on the project: an unknown project is unmeasured", () => {
    expect(judgeOutcome(TABLE, "project-b", "win32", "blocked", false).kind).toBe("unmeasured-annotate");
  });

  it("fails an unmeasured cell on CI, carrying the observed value", () => {
    const v = judgeOutcome(TABLE, "project-a", "darwin", "certificate-error", true);
    expect(v.kind).toBe("unmeasured-fail");
    expect((v as { message: string }).message).toContain('"certificate-error"');
    expect((v as { message: string }).message).toContain("darwin");
  });
});

describe("judgeAddress", () => {
  it("runs on a private address everywhere", () => {
    expect(judgeAddress("private", false)).toEqual({ kind: "ok" });
    expect(judgeAddress("private", true)).toEqual({ kind: "ok" });
  });

  it("skips a CGNAT, public or link-local address locally and fails it on CI", () => {
    for (const cls of ["cgnat", "public", "link-local", "loopback"] as const) {
      expect(judgeAddress(cls, false).kind).toBe("skip");
      const failed = judgeAddress(cls, true);
      expect(failed.kind).toBe("fail");
      expect((failed as { message: string }).message).toContain(cls);
    }
  });
});
