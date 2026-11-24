import { DEFAULT_LONG_PRESS_MS } from "@shadowcat/core";
import type { Locator, Page } from "@playwright/test";
import { test, expect, login, openPanel } from "./fixtures";

// Pointer and touch drags in the three lists that reorder or file by dragging: the note tree, the
// asset browser's folder tree and the asset grid. Mouse drags use coordinate pointer events; touch
// drags are real touch input through the browser's own input channel, held past the long-press
// threshold before they move, because a touch pointer only arms a drag after that hold.

// A 1x1 PNG, uploaded as an in-memory buffer.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

/** The hold a touch must exceed before the drag controller arms, plus a margin for timer slack. */
const TOUCH_HOLD_MS = DEFAULT_LONG_PRESS_MS + 250;

/** Steps in which a drag's pointer travels from its start to its drop point. */
const DRAG_STEPS = 8;

/** A viewport point. */
interface Point {
  /** Horizontal position in CSS pixels. */
  x: number;
  /** Vertical position in CSS pixels. */
  y: number;
}

test.use({ viewport: { width: 1600, height: 1000 }, hasTouch: true });

/** The centre of an element's box.
 * @param locator The element to measure.
 * @returns The centre point.
 * @example
 * ```
 * declare const locator: import("@playwright/test").Locator;
 * const at = await centre(locator);
 * ```
 */
async function centre(locator: Locator): Promise<Point> {
  const box = await locator.boundingBox();
  if (box === null) throw new Error("element has no box");
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** The point at `fraction` of an element's height, on its horizontal centre line.
 * @param locator The element to measure.
 * @param fraction 0 is the top edge, 1 the bottom edge.
 * @returns The point.
 * @example
 * ```
 * declare const locator: import("@playwright/test").Locator;
 * const lowerQuarter = await atHeight(locator, 0.9);
 * ```
 */
async function atHeight(locator: Locator, fraction: number): Promise<Point> {
  const box = await locator.boundingBox();
  if (box === null) throw new Error("element has no box");
  return { x: box.x + box.width / 2, y: box.y + box.height * fraction };
}

/** Drags with the mouse from one point to another in real pointer events.
 * @param page The page to drive.
 * @param from The press point.
 * @param to The release point.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await mouseDrag(page, { x: 10, y: 10 }, { x: 10, y: 90 });
 * ```
 */
async function mouseDrag(page: Page, from: Point, to: Point): Promise<void> {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: DRAG_STEPS });
  await page.mouse.up();
}

/** Drags with one finger: touch down, hold past the long-press threshold, travel, lift.
 * @param page The page to drive.
 * @param from The touch-down point.
 * @param to The lift point.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await touchDrag(page, { x: 10, y: 10 }, { x: 10, y: 90 });
 * ```
 */
async function touchDrag(page: Page, from: Point, to: Point): Promise<void> {
  const cdp = await page.context().newCDPSession(page);
  try {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [from] });
    await page.waitForTimeout(TOUCH_HOLD_MS);
    for (let step = 1; step <= DRAG_STEPS; step++) {
      const at = { x: from.x + ((to.x - from.x) * step) / DRAG_STEPS, y: from.y + ((to.y - from.y) * step) / DRAG_STEPS };
      await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [at] });
    }
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  } finally {
    await cdp.detach();
  }
}

/** Both input methods, so every list is exercised with each.
 * @example
 * ```
 * const names = INPUTS.map((input) => input.name);
 * ```
 */
const INPUTS: readonly {
  /** The input method's name, used in test titles. */
  name: string;
  /** Performs the drag. */
  drag: (page: Page, from: Point, to: Point) => Promise<void>;
}[] = [
  { name: "pointer", drag: mouseDrag },
  { name: "touch long-press", drag: touchDrag },
];

/** Creates and enters a fresh world.
 * @param page The page to drive.
 * @param account The worker's account.
 * @param account.username The username.
 * @param account.password The password.
 * @param name The world's name.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await enterWorld(page, { username: "u", password: "p" }, "Drag World");
 * ```
 */
async function enterWorld(
  page: Page,
  account: { username: string; password: string },
  name: string,
): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
}

/** Creates notes named `names` in order, closing each sheet the create opens.
 * @param page The page whose notes panel is open.
 * @param names The note names.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await createNotes(page, ["One", "Two"]);
 * ```
 */
async function createNotes(page: Page, names: string[]): Promise<void> {
  for (const name of names) {
    await page.getByTestId("notes-name").fill(name);
    await page.getByTestId("notes-create").click();
    const sheet = page.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: name });
    await expect(sheet).toBeVisible({ timeout: 15_000 });
    await sheet.getByRole("button", { name: "Close" }).click();
    await expect(sheet).toHaveCount(0);
  }
  await expect(page.getByTestId("note-open")).toHaveCount(names.length);
}

/** The note names in tree order.
 * @param page The page showing the notes panel.
 * @returns The names, top to bottom.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const order = await noteOrder(page);
 * ```
 */
async function noteOrder(page: Page): Promise<string[]> {
  return page.getByTestId("note-open").allInnerTexts();
}

for (const input of INPUTS) {
  test(`note tree: a ${input.name} drag moves a note below its last sibling`, async ({ page, account }) => {
    await enterWorld(page, account, `Note Drag ${input.name} ${Date.now().toString(36)}`);
    await openPanel(page, "notes:panel");
    await createNotes(page, ["First", "Second", "Third"]);
    expect(await noteOrder(page)).toEqual(["First", "Second", "Third"]);

    const handle = page.getByTestId("note-drag-handle").first();
    const lastRow = page.getByTestId("note-row").last().locator(".row");
    await input.drag(page, await centre(handle), await atHeight(lastRow, 0.8));

    await expect.poll(() => noteOrder(page)).toEqual(["Second", "Third", "First"]);
  });
}

test("note tree: Alt+ArrowDown and Alt+ArrowUp move the focused note one place", async ({ page, account }) => {
  await enterWorld(page, account, `Note Keys ${Date.now().toString(36)}`);
  await openPanel(page, "notes:panel");
  await createNotes(page, ["First", "Second", "Third"]);

  await page.getByTestId("note-drag-handle").first().focus();
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => noteOrder(page)).toEqual(["Second", "First", "Third"]);

  await page.getByTestId("note-drag-handle").nth(1).focus();
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(() => noteOrder(page)).toEqual(["First", "Second", "Third"]);
});

/** Opens the asset browser in a fresh world and creates the named root folders.
 * @param page The page to drive.
 * @param account The worker's account.
 * @param account.username The username.
 * @param account.password The password.
 * @param world The world's name.
 * @param folders The folder names to create, in order.
 * @returns The asset browser's folder tree.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const tree = await openFolderTree(page, { username: "u", password: "p" }, "World", ["alpha"]);
 * ```
 */
async function openFolderTree(
  page: Page,
  account: { username: string; password: string },
  world: string,
  folders: string[],
): Promise<Locator> {
  await enterWorld(page, account, world);
  await openPanel(page, "asset-browser:panel");
  await expect(page.getByTestId("asset-browser")).toBeVisible();
  const tree = page.getByTestId("asset-browser-tree");
  // The tables panel is open by default with its own folder-create input of the same test id.
  const createName = tree.getByTestId("folder-create-name");
  for (const name of folders) {
    await createName.fill(name);
    await createName.press("Enter");
    await expect(tree.locator(".row", { hasText: name })).toHaveCount(1);
  }
  return tree;
}

/** A folder row of the asset browser's tree, by folder name.
 * @param tree The tree.
 * @param name The folder's name.
 * @returns The row.
 * @example
 * ```
 * declare const tree: import("@playwright/test").Locator;
 * const row = folderRow(tree, "alpha");
 * ```
 */
function folderRow(tree: Locator, name: string): Locator {
  return tree.locator(".row", { hasText: name });
}

/** The folder names in tree order, top to bottom.
 * @param tree The tree.
 * @returns The names.
 * @example
 * ```
 * declare const tree: import("@playwright/test").Locator;
 * const order = await folderOrder(tree);
 * ```
 */
async function folderOrder(tree: Locator): Promise<string[]> {
  const texts = await tree.locator(".row").allInnerTexts();
  return texts.map((text) => /📁\s*(\S+)/.exec(text)?.[1] ?? "");
}

for (const input of INPUTS) {
  test(`folder tree: a ${input.name} drag on the lower edge of a row reorders, on its middle half nests`, async ({
    page,
    account,
  }) => {
    const tree = await openFolderTree(page, account, `Folder Drag ${input.name} ${Date.now().toString(36)}`, [
      "alpha",
      "beta",
      "gamma",
    ]);
    expect(await folderOrder(tree)).toEqual(["alpha", "beta", "gamma"]);

    // Bottom edge of the last row: placement "after", so alpha lands last.
    await input.drag(
      page,
      await centre(folderRow(tree, "alpha").locator('[data-testid^="folder-drag-"]')),
      await atHeight(folderRow(tree, "gamma"), 0.95),
    );
    await expect.poll(() => folderOrder(tree)).toEqual(["beta", "gamma", "alpha"]);

    // Middle of beta's row: placement "into", so gamma becomes beta's child.
    await input.drag(
      page,
      await centre(folderRow(tree, "gamma").locator('[data-testid^="folder-drag-"]')),
      await atHeight(folderRow(tree, "beta"), 0.5),
    );
    await expect(folderRow(tree, "gamma")).toHaveAttribute("style", /padding-left: 0.75rem/);
    await expect(folderRow(tree, "beta")).toHaveAttribute("style", /padding-left: 0rem/);
  });

  test(`asset grid: a ${input.name} drag of a tile onto a folder row files it, onto the all-assets row takes it out`, async ({
    page,
    account,
  }) => {
    const tree = await openFolderTree(page, account, `Asset Drag ${input.name} ${Date.now().toString(36)}`, [
      "alpha",
      "beta",
    ]);
    await page
      .getByTestId("asset-upload-input")
      .setInputFiles({ name: "token.png", mimeType: "image/png", buffer: PNG_1X1 });
    const tile = page.getByTestId("asset-tile");
    await expect(tile).toHaveCount(1);

    // The middle of the folder's row: the tile files into alpha.
    await input.drag(page, await centre(tile), await centre(folderRow(tree, "alpha")));
    await folderRow(tree, "alpha").getByTestId(/folder-node-/).click();
    await expect(tile).toHaveCount(1);
    await folderRow(tree, "beta").getByTestId(/folder-node-/).click();
    await expect(page.getByTestId("asset-browser-empty")).toBeVisible();

    // Back in alpha, the tile dragged onto the all-assets row (the root drop target) leaves it.
    await folderRow(tree, "alpha").getByTestId(/folder-node-/).click();
    await expect(tile).toHaveCount(1);
    await input.drag(page, await centre(tile), await centre(tree.getByTestId("folder-node-all")));
    await expect(page.getByTestId("asset-browser-empty")).toBeVisible();
    await tree.getByTestId("folder-node-all").click();
    await expect(tile).toHaveCount(1);
    await folderRow(tree, "alpha").getByTestId(/folder-node-/).click();
    await expect(page.getByTestId("asset-browser-empty")).toBeVisible();
  });
}
