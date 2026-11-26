import { describe, it, expect } from "vitest";
import { fmtBytes } from "./format";

describe("fmtBytes", () => {
  it.each([
    [0, "0 B"],
    [1, "1 B"],
    [1023, "1023 B"],
    [1024, "1.0 KiB"],
    [1536, "1.5 KiB"],
    [1024 * 1024 - 1, "1024.0 KiB"],
    [1024 * 1024, "1.0 MiB"],
    [Math.round(1024 * 1024 * 3.4), "3.4 MiB"],
  ])("formats %s as %s", (input, expected) => {
    expect(fmtBytes(input)).toBe(expected);
  });

  it("formats a bigint exactly as the equal number", () => {
    expect(fmtBytes(2048n)).toBe(fmtBytes(2048));
    expect(fmtBytes(2048n)).toBe("2.0 KiB");
    expect(fmtBytes(3n * 1024n * 1024n)).toBe("3.0 MiB");
  });
});
