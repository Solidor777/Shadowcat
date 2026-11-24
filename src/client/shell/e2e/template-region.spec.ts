import type { Page } from "@playwright/test";
import { test, expect, login } from "./fixtures";
import { clickScene, dragScene, sceneCenter } from "./stage-gestures";

// A GM draws a circle template, selects it with the select tool and presses "Make this a
// region". The template is replaced by a region in one batch: the shape count (drawings +
// templates) drops to zero and the rail opens the region editor, which only renders for a
// region document present in the optimistic store.
test("a GM turns a drawn template into a region", async ({ page, account }) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Template Region World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const center = await sceneCenter(page);
  await page.getByTestId("tool-template").click();
  await dragScene(page, center, { x: center.x + 60, y: center.y }, 3);
  await expect(host).toHaveAttribute("data-shape-count", "1", { timeout: 15_000 });

  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await expect(page.getByTestId("template-editor")).toBeVisible();

  await page.getByTestId("template-make-region").click();

  await expect(page.getByTestId("region-editor")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("template-editor")).toHaveCount(0);
  await expect(host).toHaveAttribute("data-shape-count", "0", { timeout: 15_000 });
});

/** The documents of one type in the named world, read through the documents API of the page's session.
 * @param page A page signed in as a member of the world.
 * @param worldName The world's name.
 * @param type The document type to list.
 * @returns The documents.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const regions = await worldDocs(page, "World", "region");
 * ```
 */
async function worldDocs(page: Page, worldName: string, type: string): Promise<WireDoc[]> {
  return page.evaluate(
    async ({ name, kind }) => {
      const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
      const world = worlds.find((w) => w.name === name);
      if (!world) throw new Error(`world ${name} not listed`);
      return (await (await fetch(`/api/worlds/${world.id}/documents?type=${kind}`)).json()) as WireDoc[];
    },
    { name: worldName, kind: type },
  );
}

/** The world's latest committed sequence number, read from its snapshot.
 * @param page A page signed in as a member of the world.
 * @param worldName The world's name.
 * @returns The sequence number.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const seq = await worldSeq(page, "World");
 * ```
 */
async function worldSeq(page: Page, worldName: string): Promise<number> {
  return page.evaluate(async (name) => {
    const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
    const world = worlds.find((w) => w.name === name);
    if (!world) throw new Error(`world ${name} not listed`);
    return ((await (await fetch(`/api/worlds/${world.id}/snapshot`)).json()) as { seq: number }).seq;
  }, worldName);
}

/** The slice of a document these tests read. */
interface WireDoc {
  /** The document id. */
  id: string;
  /** The parent document id, if any. */
  parent_id: string | null;
  /** The typed engine band. */
  engine: { shape: { kind: string; x?: number; y?: number; size?: number; points?: number[] } };
}

// The conversion is ONE committed command (the region's create and the template's delete share a
// batch, so one history entry), and the region it leaves is the template's own area: a circle
// template at (x, y) of size r becomes a circle region at (x, y) of radius r.
test("making a template a region commits one batch, and the region covers the template's area", async ({
  page,
  account,
}) => {
  const worldName = `Template Region Batch ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const center = await sceneCenter(page);
  await page.getByTestId("tool-template").click();
  await dragScene(page, center, { x: center.x + 60, y: center.y }, 3);
  await expect(host).toHaveAttribute("data-shape-count", "1", { timeout: 15_000 });
  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await expect(page.getByTestId("template-editor")).toBeVisible();

  const [template] = await worldDocs(page, worldName, "template");
  expect(template.engine.shape.kind).toBe("circle");
  const before = await worldSeq(page, worldName);

  await page.getByTestId("template-make-region").click();
  await expect(page.getByTestId("region-editor")).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => worldDocs(page, worldName, "template")).toHaveLength(0);

  // One batch: the sequence advanced by exactly one, so the create and the delete are one entry.
  expect(await worldSeq(page, worldName)).toBe(before + 1);

  const regions = await worldDocs(page, worldName, "region");
  expect(regions).toHaveLength(1);
  expect(regions[0].parent_id).toBe(template.parent_id);
  expect(regions[0].engine.shape).toEqual({
    kind: "circle",
    points: [template.engine.shape.x, template.engine.shape.y, template.engine.shape.size],
  });
});
