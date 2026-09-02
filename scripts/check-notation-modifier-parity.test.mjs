import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";
import {
  extractModifierMatchBlock,
  extractRustModifierIdents,
  extractRustNotationFunctions,
  extractRustTemplateFunctions,
  extractRustTemplateKeywords,
  modifierParityDifference,
} from "./check-notation-modifier-parity.mjs";
import { NOTATION_FUNCTIONS, NOTATION_KEYWORDS } from "../src/client/formula/src/template.ts";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const rustSource = readFileSync(
  join(repoRoot, "src", "server", "src", "dice", "notation", "parser.rs"),
  "utf8",
);
const rustTemplateSource = readFileSync(
  join(repoRoot, "src", "server", "src", "formula", "template.rs"),
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

// The keyword vocabulary is ONE decision with THREE declarations: the server's notation
// parser arms, the client template module's list, and the server formula twin's list. None
// of the three can read another, so each pair is diffed here.

test("the server formula twin reserves exactly the client template module's list", () => {
  expect(extractRustTemplateKeywords(rustTemplateSource)).toEqual(NOTATION_KEYWORDS);
});

test("the server formula twin's list is checked against the parser arms too", () => {
  const rustIdents = extractRustModifierIdents(rustSource);
  expect(modifierParityDifference(rustIdents, extractRustTemplateKeywords(rustTemplateSource),
    DICE_OPERATOR)).toEqual({ missingFromTemplate: [], unknownToServer: [] });
});

test("the template-twin extractor resolves the dice-operator constant and rejects junk", () => {
  const source = `const DICE_OPERATOR: &str = "d";\n` +
    `pub(crate) const NOTATION_KEYWORDS: [&str; 3] = [DICE_OPERATOR, "kh", "kl"];`;
  expect(extractRustTemplateKeywords(source)).toEqual(["d", "kh", "kl"]);
  expect(() => extractRustTemplateKeywords(`const NOTATION_KEYWORDS: [&str; 1] = ["kh"];`))
    .toThrow(/DICE_OPERATOR/);
  expect(() => extractRustTemplateKeywords(source.replace("NOTATION_KEYWORDS", "RENAMED")))
    .toThrow(/NOTATION_KEYWORDS/);
  expect(() => extractRustTemplateKeywords(source.replace(`"kh"`, "kh")))
    .toThrow(/unrecognized entry/);
});

// The math-function vocabulary is likewise ONE decision with THREE declarations: the dice
// parser's `fn_call` match arms and both template sides' `NOTATION_FUNCTIONS`.

test("the template grammar reserves exactly the math functions the notation parser accepts", () => {
  const parserFunctions = extractRustNotationFunctions(rustSource);
  expect(parserFunctions.length).toBeGreaterThan(0);
  expect(NOTATION_FUNCTIONS).toEqual(parserFunctions);
  expect(extractRustTemplateFunctions(rustTemplateSource)).toEqual(parserFunctions);
});

test("the function extractors fail loudly on renames and junk", () => {
  expect(() => extractRustNotationFunctions("fn other() { match x { } }"))
    .toThrow(/fn fn_call/);
  expect(() => extractRustNotationFunctions(
    `fn fn_call(&mut self) { match name.as_str() { "floor" => f(), } }`))
    .toThrow(/catch-all/);
  expect(() => extractRustTemplateFunctions(`const NOTATION_FUNCTIONS: [&str; 1] = [floor];`))
    .toThrow(/unrecognized entry/);
  expect(() => extractRustTemplateFunctions(`const RENAMED: [&str; 1] = ["floor"];`))
    .toThrow(/NOTATION_FUNCTIONS/);
});
