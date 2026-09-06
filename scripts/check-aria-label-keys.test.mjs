import { test, expect } from "vitest";
import { scanAriaLabelKeys, isComponentSource, HUMAN_ATTRS } from "./check-aria-label-keys.mjs";

test("a literal i18n key as an aria-label is a violation", () => {
  const src = '<label>\n  {ctx.t("gameSettings.scene.gridSize")}\n  <input type="number" aria-label="gameSettings.scene.gridSize" />\n</label>\n';
  expect(scanAriaLabelKeys(src)).toEqual([{ line: 3, attr: "aria-label", value: "gameSettings.scene.gridSize" }]);
});

test("a key carrying a template interpolation is a violation", () => {
  const src = '<input aria-label="gameSettings.gradation.{band.name}" />\n';
  expect(scanAriaLabelKeys(src)).toEqual([{ line: 1, attr: "aria-label", value: "gameSettings.gradation.{band.name}" }]);
});

test("an expression whose first token is a key literal is a violation, including a concatenation prefix", () => {
  const src = '<input aria-label={"gameSettings.resources.name-" + key} />\n<select aria-label={"gameSettings.combat.scene." + leaf}></select>\n';
  expect(scanAriaLabelKeys(src).map((v) => [v.line, v.value])).toEqual([
    [1, "gameSettings.resources.name-"],
    [2, "gameSettings.combat.scene."],
  ]);
});

test("a template literal beginning with a key is a violation, judged on the text before its first substitution", () => {
  const src = "<input aria-label={`gameSettings.visionMode.${mode.id}.range`} />\n";
  expect(scanAriaLabelKeys(src).map((v) => v.line)).toEqual([1]);
  expect(scanAriaLabelKeys("<input aria-label={`${prefix} range`} />\n")).toEqual([]);
});

test("a single-quoted value is read the same as a double-quoted one", () => {
  expect(scanAriaLabelKeys("<input aria-label='combatTracker.notation' />\n").map((v) => v.value)).toEqual(["combatTracker.notation"]);
  expect(scanAriaLabelKeys("<input aria-label={'combatTracker.notation'} />\n").map((v) => v.value)).toEqual(["combatTracker.notation"]);
});

test("every human-facing attribute in the family is covered, not only aria-label", () => {
  for (const attr of HUMAN_ATTRS) {
    expect(scanAriaLabelKeys(`<input ${attr}="chat.composer.placeholder" />\n`).map((v) => v.attr)).toEqual([attr]);
  }
});

test("a resolved key, human text, and text with a dot and a space all pass", () => {
  const src = [
    '<input aria-label={ctx.t("gameSettings.scene.gridSize")} />',
    '<input aria-label={t("gameSettings.visionModeRangeFor", { id: mode.id })} />',
    '<input aria-label="Grid cell size (pixels)" />',
    '<button title="Reset to default." />',
    '<input placeholder="e.g. 1d20 + 5" />',
    '<input aria-label="Version 1.2" />',
    "<input aria-label={labelText} />",
  ].join("\n");
  expect(scanAriaLabelKeys(src)).toEqual([]);
});

test("a value that only LOOKS like a key because of a dotted identifier tail still fails when it has no whitespace", () => {
  expect(scanAriaLabelKeys('<input title="v1.2" />\n').map((v) => v.value)).toEqual(["v1.2"]);
});

test("a data attribute or a non-human attribute with the same shape is not a violation", () => {
  const src = '<p data-testid="provenance:scene.fog" class="a.b" id="x.y" value="opt.one"></p>\n';
  expect(scanAriaLabelKeys(src)).toEqual([]);
});

test("a match inside a markup comment or a script comment is prose", () => {
  const src = '<!-- aria-label="gameSettings.scene.gridSize" is the old shape -->\n<script>\n  // aria-label="a.b"\n  /* title="c.d" */\n</script>\n';
  expect(scanAriaLabelKeys(src)).toEqual([]);
});

test("a value wrapped onto the line after its equals sign is still found, and reported at the attribute's line", () => {
  const src = '<input\n  aria-label=\n    "combatTracker.initiative"\n/>\n';
  expect(scanAriaLabelKeys(src)).toEqual([{ line: 2, attr: "aria-label", value: "combatTracker.initiative" }]);
});

test("several attributes on one line are each reported", () => {
  const src = '<input aria-label="a.b" placeholder="c.d" title="Real title" />\n';
  expect(scanAriaLabelKeys(src).map((v) => v.attr)).toEqual(["aria-label", "placeholder"]);
});

test("isComponentSource covers tracked .svelte under src and examples only", () => {
  expect(isComponentSource("src/modules/game-settings/src/GameSettingsPanel.svelte")).toBe(true);
  expect(isComponentSource("examples\\module-scaffold\\src\\Panel.svelte")).toBe(true);
  expect(isComponentSource("src/modules/game-settings/src/index.ts")).toBe(false);
  expect(isComponentSource("docs/site/components/Demo.svelte")).toBe(false);
});
