import { readFileSync } from "node:fs";
import { randomUuid } from "@shadowcat/core";
import { fileURLToPath } from "node:url";
import { test, expect, login, openPanel } from "./fixtures";
import type { Page } from "@playwright/test";

// Media pages of the note sheet. A note whose `engine.kind` is an image, a PDF or a video renders
// that asset in place of a text body: an `<img>`, an `<embed type="application/pdf">` and a
// `<video>`. When the asset is deleted the sheet shows the missing-media placeholder instead, and
// "Change media" swaps an image page to another asset of the same kind. The UI has no control that
// creates a media page, so the notes are created through the documents API by cloning a note the
// UI made (the same document shape the server validates on ingress).

const PNG_A = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);
// A 2x2 red PNG: different bytes, so the second image is a distinct asset.
const PNG_B = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAACXBIWXMAAAABAAAAAQBPJcTWAAAAEElEQVR4nGP8ywACLGCSAQANEQED1LYyQAAAAABJRU5ErkJggg==",
  "base64",
);
const PDF = Buffer.from(
  "%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n" +
    "2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n" +
    "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 72 72]>>endobj\n" +
    "trailer<</Root 1 0 R/Size 4>>\n%%EOF\n",
  "latin1",
);
const WEBM = readFileSync(fileURLToPath(new URL("../e2e-fixtures/media/note-clip.webm", import.meta.url)));

/** The kind of a media page and the asset it points at. */
interface MediaKind {
  /** The page kind. */
  kind: "image" | "pdf" | "video";
  /** The asset id the page shows. */
  asset_id: string;
}

/** One uploaded asset as the asset list reports it. */
interface AssetRow {
  /** The asset id. */
  id: string;
  /** The filename as uploaded. */
  original_name: string;
}

/** Uploads `file` through the asset browser's file input and waits for it to land.
 * @param page The GM's page with the asset browser open.
 * @param file The upload's name, MIME type and bytes.
 * @param expectedCount The tile count once the upload has landed.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await upload(page, { name: "a.png", mimeType: "image/png", buffer: Buffer.alloc(0) }, 1);
 * ```
 */
async function upload(
  page: Page,
  file: { name: string; mimeType: string; buffer: Buffer },
  expectedCount: number,
): Promise<void> {
  await page.getByTestId("asset-upload-input").setInputFiles(file);
  await expect(page.getByTestId("asset-tile")).toHaveCount(expectedCount, { timeout: 30_000 });
}

/** Lists the world's assets through the REST list.
 * @param page A page signed in as a member of the world.
 * @param worldName The world's name.
 * @returns The assets.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const assets = await listAssets(page, "World");
 * ```
 */
async function listAssets(page: Page, worldName: string): Promise<AssetRow[]> {
  return page.evaluate(async (name) => {
    const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
    const world = worlds.find((w) => w.name === name);
    if (!world) throw new Error(`world ${name} not listed`);
    return (await (await fetch(`/api/worlds/${world.id}/assets`)).json()) as AssetRow[];
  }, worldName);
}

/** Creates a media note by cloning the world's first note and replacing its name and page kind.
 * @param page The GM's page.
 * @param worldName The world's name.
 * @param name The new note's name.
 * @param kind The media page kind.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await createMediaNote(page, "World", "Poster", { kind: "image", asset_id: "00000000-0000-0000-0000-000000000000" });
 * ```
 */
async function createMediaNote(page: Page, worldName: string, name: string, kind: MediaKind): Promise<void> {
  const status = await page.evaluate(
    async ({ worldName: world, name: title, kind: pageKind, id }) => {
      const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
      const found = worlds.find((w) => w.name === world);
      if (!found) throw new Error(`world ${world} not listed`);
      const notes = (await (await fetch(`/api/worlds/${found.id}/documents?type=note`)).json()) as {
        id: string;
        name: string | null;
        engine: Record<string, unknown>;
      }[];
      const clone = { ...notes[0], id, name: title, engine: { ...notes[0].engine, kind: pageKind } };
      const response = await fetch(`/api/worlds/${found.id}/documents`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(clone),
      });
      return response.status;
    },
    { worldName, name, kind, id: randomUuid() },
  );
  expect(status).toBeLessThan(300);
}

/** Reads the page kind of the named note from the documents API.
 * @param page The GM's page.
 * @param worldName The world's name.
 * @param name The note's name.
 * @returns The stored page kind.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const kind = await storedKind(page, "World", "Poster");
 * ```
 */
async function storedKind(page: Page, worldName: string, name: string): Promise<MediaKind> {
  return page.evaluate(
    async ({ worldName: world, name: title }) => {
      const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
      const found = worlds.find((w) => w.name === world);
      if (!found) throw new Error(`world ${world} not listed`);
      const notes = (await (await fetch(`/api/worlds/${found.id}/documents?type=note`)).json()) as {
        name: string | null;
        engine: { kind: MediaKind };
      }[];
      const note = notes.find((n) => n.name === title);
      if (!note) throw new Error(`note ${title} not listed`);
      return note.engine.kind;
    },
    { worldName, name },
  );
}

test("note media: image, PDF and video pages render, a deleted asset shows the placeholder, and Change media swaps the image", async ({
  page,
  account,
}) => {
  const worldName = `Note Media World ${Date.now().toString(36)}`;

  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openPanel(page, "asset-browser:panel");
  await upload(page, { name: "first.png", mimeType: "image/png", buffer: PNG_A }, 1);
  await upload(page, { name: "second.png", mimeType: "image/png", buffer: PNG_B }, 2);
  await upload(page, { name: "paper.pdf", mimeType: "application/pdf", buffer: PDF }, 3);
  await upload(page, { name: "clip.webm", mimeType: "video/webm", buffer: WEBM }, 4);
  const assets = await listAssets(page, worldName);
  const idOf = (name: string): string => {
    const found = assets.find((a) => a.original_name === name);
    if (!found) throw new Error(`asset ${name} not uploaded`);
    return found.id;
  };

  // One ordinary note supplies the document shape every media note clones.
  await openPanel(page, "notes:panel");
  await page.getByTestId("notes-name").fill("Template note");
  await page.getByTestId("notes-create").click();
  await expect(page.getByTestId("note-row")).toHaveCount(1, { timeout: 15_000 });

  await createMediaNote(page, worldName, "Poster", { kind: "image", asset_id: idOf("first.png") });
  await createMediaNote(page, worldName, "Handout", { kind: "pdf", asset_id: idOf("paper.pdf") });
  await createMediaNote(page, worldName, "Recording", { kind: "video", asset_id: idOf("clip.webm") });
  await expect(page.getByTestId("note-row")).toHaveCount(4, { timeout: 15_000 });

  /** Opens the named note's sheet from the notes panel.
   * @param name The note's name.
   * @returns The sheet dialog.
   */
  const openNote = async (name: string) => {
    await page.getByTestId("note-row").filter({ hasText: name }).getByTestId("note-open").click();
    const sheet = page.getByRole("dialog", { name: "Sheet", exact: true }).filter({
      has: page.getByRole("heading", { name, level: 2 }),
    });
    await expect(sheet).toBeVisible({ timeout: 15_000 });
    return sheet;
  };

  // Image: the element decodes the asset.
  const poster = await openNote("Poster");
  const image = poster.getByTestId("note-media").locator("img");
  await expect(poster.getByTestId("note-media")).toHaveAttribute("data-media-kind", "image");
  await expect(image).toBeVisible();
  await expect.poll(() => image.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);

  // PDF: the embed points at the stored asset and no placeholder shows.
  const handout = await openNote("Handout");
  await expect(handout.getByTestId("note-media")).toHaveAttribute("data-media-kind", "pdf");
  await expect(handout.locator('embed[type="application/pdf"]')).toHaveAttribute("src", new RegExp(idOf("paper.pdf")));
  await expect(handout.getByTestId("note-media-missing")).toHaveCount(0);
  // Floating sheets stack over one another; each is closed once checked so a later one is clickable.
  await handout.getByRole("button", { name: "Close" }).click();
  await expect(handout).toBeHidden();

  // Video: the element loads the clip's metadata.
  const recording = await openNote("Recording");
  const video = recording.getByTestId("note-media").locator("video");
  await expect(recording.getByTestId("note-media")).toHaveAttribute("data-media-kind", "video");
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  await expect(recording.getByTestId("note-media-missing")).toHaveCount(0);
  await recording.getByRole("button", { name: "Close" }).click();
  await expect(recording).toBeHidden();

  // Change media: the image page is repointed at the second image.
  await poster.getByTestId("note-change-media").click();
  const pickDialog = page.getByTestId("asset-pick-dialog");
  await expect(pickDialog).toBeVisible();
  await pickDialog.locator('[data-testid="asset-tile"][title="second.png"]').click();
  await page.getByTestId("pick-confirm").click();
  await expect.poll(() => storedKind(page, worldName, "Poster")).toEqual({
    kind: "image",
    asset_id: idOf("second.png"),
  });
  await expect(poster.getByTestId("note-media").locator("img")).toHaveAttribute("src", new RegExp(idOf("second.png")));

  // Deleting the video asset swaps its page for the placeholder.
  const status = await page.evaluate(async (id) => (await fetch(`/api/assets/${id}`, { method: "DELETE" })).status, idOf("clip.webm"));
  expect(status).toBeLessThan(300);
  const reopened = await openNote("Recording");
  await expect(reopened.getByTestId("note-media-missing")).toBeVisible({ timeout: 15_000 });
  await expect(reopened.getByTestId("note-media").locator("video")).toHaveCount(0);
});
