// @vitest-environment node
import { describe, expect, it } from "vitest";
import { presentMembers, SECURE_CONTEXT_MEMBERS } from "./secure-context";

/** The `"<owner>.<member>"` key of the table's `index`th entry for `owner`; keys are derived from
 * the table so this file never spells a gated member's name. */
function key(owner: keyof typeof SECURE_CONTEXT_MEMBERS, index: number): string {
  return `${owner}.${SECURE_CONTEXT_MEMBERS[owner][index]}`;
}

describe("presentMembers", () => {
  it("lists exactly the table entries whose typeof is not undefined, sorted", () => {
    const present = [key("navigator", 1), key("navigator", 0)];
    expect(
      presentMembers({
        isSecureContext: "true",
        [present[0]]: "function",
        [present[1]]: "object",
        [key("crypto", 0)]: "undefined",
        "crypto.getRandomValues": "function",
      }),
    ).toEqual([...present].sort());
  });

  it("treats an entry missing from the measurement as absent", () => {
    expect(presentMembers({ isSecureContext: "false" })).toEqual([]);
  });

  it("carries a second-level entry, which the plain-HTTP and HTTPS specs share", () => {
    expect(SECURE_CONTEXT_MEMBERS.navigator.some((name) => name.includes("."))).toBe(true);
  });
});
