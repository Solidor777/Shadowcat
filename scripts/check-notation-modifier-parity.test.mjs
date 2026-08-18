import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";
import {
  extractModifierMatchBlock,
  extractRustModifierIdents,
  modifierParityDifference,
} from "./check-notation-modifier-parity.mjs";
import { NOTATION_KEYWORDS } from "../src/client/formula/src/template.ts";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const rustSource = readFileSync(
  join(repoRoot, "src", "server", "src", "dice", "notation", "parser.rs"),
  "utf8",
);

// The dice operator is a notation token, not a modifier: `P::modifiers` never matches it,
// and the template grammar reserves it because a bare `d` in a template is dice notation.
const DICE_OPERATOR = "d";

test("the template grammar reserves exactly the modifiers the server's parser accepts", () => {
  const rustIdents = extractRustModifierIdents(rustSource);
  expect(rustIdents.length).toBeGreaterThan(0);
  expect(modifierParityDifference(rustIdents, NOTATION_KEYWORDS, DICE_OPERATOR))
    .toEqual({ missingFromTemplate: [], unknownToServer: [] });
});

test("the dice operator is reserved by the template grammar and matched by no modifier arm", () => {
  expect(NOTATION_KEYWORDS).toContain(DICE_OPERATOR);
  expect(extractRustModifierIdents(rustSource)).not.toContain(DICE_OPERATOR);
});

test("a modifier the template grammar does not reserve is reported", () => {
  expect(modifierParityDifference(["kh", "xz"], ["d", "kh"], "d"))
    .toEqual({ missingFromTemplate: ["xz"], unknownToServer: [] });
});

test("a reserved word the server's parser does not accept is reported", () => {
  expect(modifierParityDifference(["kh"], ["d", "kh", "zz"], "d"))
    .toEqual({ missingFromTemplate: [], unknownToServer: ["zz"] });
});

test("an alternation arm contributes each of its literals", () => {
  const source = `fn modifiers(&mut self) {
    match id.as_str() {
        "r" | "ro" => reroll(),
        "kh" => keep(),
        other => Err(format!("unknown dice modifier '{other}'")),
    }
  }`;
  expect(extractRustModifierIdents(source)).toEqual(["r", "ro", "kh"]);
});

test("a renamed anchor fails loudly rather than reporting parity over nothing", () => {
  expect(() => extractRustModifierIdents("fn other_name() { match x { } }"))
    .toThrow(/fn modifiers/);
  expect(() => extractModifierMatchBlock("fn modifiers(&mut self) { let x = 1; }"))
    .toThrow(/match id.as_str/);
  expect(() => extractModifierMatchBlock(`fn modifiers() { match id.as_str() { "kh" => k(), } }`))
    .toThrow(/catch-all/);
});
