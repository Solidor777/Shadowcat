import { test, expect } from "vitest";
import { checkBinarySize, SIZE_LIMIT_BYTES } from "./check-binary-size.mjs";

test("a size under the budget passes", () => {
  expect(checkBinarySize(SIZE_LIMIT_BYTES - 1)).toEqual({
    ok: true,
    sizeBytes: SIZE_LIMIT_BYTES - 1,
    limitBytes: SIZE_LIMIT_BYTES,
  });
});

test("a size at or over the budget fails", () => {
  expect(checkBinarySize(SIZE_LIMIT_BYTES).ok).toBe(false);
  expect(checkBinarySize(SIZE_LIMIT_BYTES + 1).ok).toBe(false);
});

test("a custom limit is honoured instead of the default", () => {
  expect(checkBinarySize(100, 200)).toEqual({ ok: true, sizeBytes: 100, limitBytes: 200 });
  expect(checkBinarySize(300, 200).ok).toBe(false);
});
