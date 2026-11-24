import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test, expect, login, newE2EContext } from "./fixtures";
import { startServer } from "../e2e-support/server-process";
import type { RunningServer } from "../e2e-support/server-process";

// A real backup-and-restore round trip. The suite's shared servers run on `sqlite::memory:`, which
// has no database file to snapshot (`backups.spec.ts` drives that real rejection), so this test
// starts a server of its own on a file-backed database in a data directory. The admin backs the
// world list up, creates a second world, and restores the backup: the server stages the restore and
// stops itself, and the next start applies it before the database opens, so the world made after the
// backup is gone and the one made before it is back.
//
// The data directory is a fresh folder under Playwright's per-test output directory, which Playwright
// clears at the start of every run, so backups never accumulate and this test makes no delete call.

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");

const ADMIN = { username: "ops", password: "pw-boot" };

test("backup and restore: a world made after a backup is gone once the restore is applied on the next start", async ({
  browser,
}, testInfo) => {
  test.setTimeout(240_000);
  // The data directory the round trip's server keeps its database, assets and backups in.
  const dataDir = path.join(testInfo.outputDir, "data");
  mkdirSync(dataDir, { recursive: true });
  const databaseUrl = `sqlite:${path.join(dataDir, "shadowcat.db").replaceAll("\\", "/")}`;
  const stamp = Date.now().toString(36);
  const keptWorld = `Kept ${stamp}`;
  const lostWorld = `Lost ${stamp}`;

  let server: RunningServer | undefined = await startServer({ repoRoot: REPO_ROOT, databaseUrl });
  try {
    // First run: back up with one world, then make a second.
    let ctx = await newE2EContext(browser, { baseURL: server.url });
    let page = await ctx.newPage();
    await login(page, ADMIN.username, ADMIN.password);
    await page.getByLabel("New world name").fill(keptWorld);
    await page.getByRole("button", { name: "Create world" }).click();
    await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await page.getByTestId("topbar-settings").click();
    await expect(page.getByRole("heading", { name: "Backups" })).toBeVisible();
    const before = await page.getByRole("button", { name: /^Restore the backup from / }).count();
    await page.getByRole("button", { name: "Back up now" }).click();
    await expect(page.getByRole("button", { name: /^Restore the backup from / })).toHaveCount(before + 1, {
      timeout: 60_000,
    });
    // The newest backup is listed first.
    const newest = page.getByRole("button", { name: /^Restore the backup from / }).first();
    const restoreLabel = await newest.getAttribute("aria-label");
    expect(restoreLabel).not.toBeNull();

    // A second world made after the backup: it is what the restore must remove.
    const created = await page.evaluate(async (name) => {
      const response = await fetch("/api/worlds", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name }),
      });
      return response.status;
    }, lostWorld);
    expect(created).toBeLessThan(300);
    const listedWorlds = (): Promise<string[]> =>
      page.evaluate(async () => ((await (await fetch("/api/worlds")).json()) as { name: string }[]).map((w) => w.name));
    expect(await listedWorlds()).toEqual(expect.arrayContaining([keptWorld, lostWorld]));

    // Restore: confirm, and the server stages the restore and stops.
    await newest.click();
    await page.getByRole("button", { name: "Restore and stop server" }).click();
    await expect(page.getByText(/is staged\. The server is stopping/).first()).toBeVisible({ timeout: 30_000 });
    // The server stops itself: the process exits cleanly without being killed.
    expect(await server.exited).toBe(0);
    await ctx.close();

    // Second run on the same data directory: the restore is applied before the database opens.
    server = await startServer({ repoRoot: REPO_ROOT, databaseUrl });
    ctx = await newE2EContext(browser, { baseURL: server.url });
    page = await ctx.newPage();
    await login(page, ADMIN.username, ADMIN.password);
    await expect(page.getByLabel("New world name")).toBeVisible({ timeout: 30_000 });
    const restoredResponse = await page.evaluate(async () => {
      const response = await fetch("/api/worlds");
      return { status: response.status, body: await response.text() };
    });
    expect(restoredResponse.status, restoredResponse.body).toBe(200);
    const restored = (JSON.parse(restoredResponse.body) as { name: string }[]).map((w) => w.name);
    expect(restored).toContain(keptWorld);
    expect(restored).not.toContain(lostWorld);
    await ctx.close();
  } finally {
    await server?.stop();
  }
});
