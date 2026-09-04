import { test, expect, login } from "./fixtures";

// A 1×1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Drives the served binary: after entering a world the Pixi canvas mounts, the
// engine reaches first-frame readiness, accepts a pan gesture, and tears down on
// leave. Real WebGL via headless chromium (SwiftShader).
test("stage canvas mounts, renders, and tears down on leave", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Render World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  const canvas = page.getByTestId("stage-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(0);
  expect(box?.height ?? 0).toBeGreaterThan(0);

  // A pan gesture must not throw (pointer events drive the camera).
  await canvas.hover();
  await page.mouse.down();
  await page.mouse.move(box!.x + 50, box!.y + 50);
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-render-ready", "true");

  // "Leave world" lives in the Settings panel, which starts launcher-closed;
  // open it from the topbar launcher first.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-settings:panel").click();
  await page.getByRole("button", { name: /leave world/i }).click();
  await expect(page.getByTestId("stage-canvas")).toHaveCount(0);
});

test("place a token via the tool rail, then drag it", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Token World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  // The Assets panel starts launcher-closed; open it from the topbar launcher
  // before uploading.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();

  // Upload an image asset (the token art).
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Activate the place tool and pick the asset in the rail's picker.
  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();

  // Click the canvas → a token document is created (optimistic) and rendered.
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  await expect(host).toHaveAttribute("data-token-count", "1", {
    timeout: 15_000,
  });

  // Drag the token with the select/move tool: it must not throw and the token persists.
  await page.getByTestId("tool-select").click();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width / 2 + 60,
    box.y + box.height / 2 + 40,
    { steps: 4 },
  );
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-token-count", "1");
});

// Opens assets+settings+actors into "right" across its lifetime, on top of
// chat's permanent default dock (4 groups total in one zone) — exercises the
// production `DockviewEngine`'s width containment past 2 groups in a zone.
test("author an animated (frame-list) actor token; it places without error", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Animated Actor World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  // The Assets panel starts launcher-closed; open it from the topbar launcher
  // before uploading.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();

  // Upload two frames for the animated actor.
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "f1.png", mimeType: "image/png", buffer: PNG_1X1 });
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "f2.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(2);

  // The Actors panel starts launcher-closed; open it. Frame picking goes
  // through the shared pick modal, which queries live — no remount dance.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-actors:panel").click();

  // Author an animated (frame-list) actor via the Actors panel. Scoped to `.actors`
  // (ActorsPanel's root section) since the Factions/Conditions panels reuse the same
  // "Name" label text — an unscoped locator would be ambiguous.
  const actorsPanel = page.locator(".actors");
  await actorsPanel.getByPlaceholder("Name", { exact: true }).fill("Wisp");
  await actorsPanel.getByLabel("Visual").selectOption("animated");
  // Ordered multi-pick through the modal: both frames, in upload order.
  await actorsPanel.getByTestId("visual-pick-frames").click();
  const pickDialog = page.getByTestId("asset-pick-dialog");
  await expect(pickDialog).toBeVisible();
  const pickTiles = pickDialog.getByTestId("asset-tile");
  await expect(pickTiles).toHaveCount(2);
  await pickTiles.nth(0).click();
  await pickTiles.nth(1).click();
  await pickDialog.getByTestId("pick-confirm").click();
  await expect(pickDialog).not.toBeVisible();
  await actorsPanel.getByLabel("Frames per second").fill("10");
  await actorsPanel.getByRole("button", { name: "Create actor" }).click();

  // Select the actor, then activate the place tool and click the canvas — mirrors
  // the existing token-placement flow. `makePlaceTool` gives a selected actor
  // precedence over a raw asset, so no asset picker is needed.
  await actorsPanel.getByRole("button", { name: "Wisp", exact: true }).click();
  await page.getByTestId("tool-place").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  await expect(host).toHaveAttribute("data-token-count", "1", {
    timeout: 15_000,
  });
});

test("draw a freehand stroke via the tool rail; the drawing renders", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Draw World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  await page.getByTestId("tool-draw").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  // Drag a freehand path across the canvas.
  await page.mouse.move(box.x + box.width / 2 - 40, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 - 30, {
    steps: 3,
  });
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2, {
    steps: 3,
  });
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-shape-count", "1", {
    timeout: 15_000,
  });
});

test("ping a location via the tool rail; the relayed ping renders", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Ping World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  await page.getByTestId("tool-ping").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  // The server relays the ping back to the sender → Stage's onPing sets data-last-ping.
  await expect(host).toHaveAttribute("data-last-ping", /.+/, {
    timeout: 15_000,
  });
});

test("draw a wall via the tool rail; the wall renders", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Wall World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  await page.getByTestId("tool-wall").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2 - 60, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width / 2 + 60,
    box.y + box.height / 2 + 20,
    { steps: 3 },
  );
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-wall-count", "1", {
    timeout: 15_000,
  });
});

test("the vision SceneDerived channel reaches the mask slot (GM mode=all)", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Vision Spike World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world subscribes to the "vision" channel; the server pushes the initial
  // frame, the engine applies it (watermark-gated) and records the fog mode. As the world
  // owner (GM) the mode is "all" (no fog) — proving the vision channel reaches the mask slot
  // end-to-end in real GL (the fog `setVisibility` path runs without error).
  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
  await expect(host).toHaveAttribute("data-scene-derived", "1", {
    timeout: 30_000,
  });
  await expect(host).toHaveAttribute("data-vision-mode", "all", {
    timeout: 30_000,
  });
});

test("GM vision dropdown: see-all / preview-fog drive the fog in real GL", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Fog View World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
  // GM default: see-all (no fog).
  await expect(host).toHaveAttribute("data-vision-mode", "all", {
    timeout: 30_000,
  });
  await expect(host).toHaveAttribute("data-gm-view", "all");

  // "Preview fog" → the player view (full fog) renders in real GL; the effective mode flips to masked.
  await page.getByTestId("gm-view-select").selectOption("fog");
  await expect(host).toHaveAttribute("data-gm-view", "fog");
  await expect(host).toHaveAttribute("data-vision-mode", "masked");

  // Back to "See all" → no fog restored.
  await page.getByTestId("gm-view-select").selectOption("all");
  await expect(host).toHaveAttribute("data-gm-view", "all");
  await expect(host).toHaveAttribute("data-vision-mode", "all");
});

// Below the 48rem/768px breakpoint the panel host switches to its
// compact presentation, which hides `.engine-host` — the stage canvas must be relocated
// into the persistent `.compact-stage` well (kept mounted, never inside a hidden ancestor)
// rather than buried and invisible.
test("compact viewport (mobile width): the stage canvas stays visible, outside any hidden ancestor", async ({
  page,
  account,
}) => {
  await page.setViewportSize({ width: 375, height: 700 });
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Compact Stage World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  await expect(page.locator(".engine-host")).toBeHidden();
  await expect(page.locator(".compact-stage")).toBeVisible();

  const canvas = page.getByTestId("stage-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(0);
  expect(box?.height ?? 0).toBeGreaterThan(0);
});

// Authors a scene background via the SceneBrowserPanel's asset picker and confirms it reaches
// the render layer (`data-background`, set from the same `engine.background` field the stage's
// background sprite paints from) — the authoring half of the render-consumption path that already
// worked before this picker existed.
test("pick a scene background via the scene browser; it reaches the stage", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Background World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
  await expect(host).toHaveAttribute("data-background", "", { timeout: 15_000 });

  // Upload the background image asset.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "bg.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // The scene-browser panel's asset picker list is fetched once at mount and has no
  // live-refresh hook for a newly-CREATED asset (only replace/delete broadcast an
  // AssetChanged — mirrors ActorsPanel/VisualKindEditor's identical picker convention
  // above); leave and re-enter the world so the panel remounts and its picker sees the
  // upload.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-settings:panel").click();
  await page.getByRole("button", { name: /leave world/i }).click();
  await page.getByRole("button", { name: /Background World/ }).click();
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  // Open the Scenes panel and pick the uploaded image as the (auto-created) scene's background.
  // Scoped to the panel's own labelled region: the Assets panel (re-docked from the layout
  // persisted across leave/re-enter) renders its own "bg.png"-accessible-named tile button, and
  // an unscoped role query would match both.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-scene-browser:panel").click();
  const scenesPanel = page.getByRole("region", { name: "Scenes" });
  await scenesPanel.getByRole("button", { name: "Set background image" }).first().click();
  const pick = scenesPanel.getByRole("button", { name: "bg.png" });
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();

  // The scene doc's engine.background is now the uploaded asset's id, and the stage's
  // observability signal (mirroring the reconciler's own read of that field) reflects it.
  await expect(host).not.toHaveAttribute("data-background", "", {
    timeout: 15_000,
  });
});

// The layout grid is `100vh` tall and its middle row is shared by the tool rail and the
// panel host. A grid row is at least as tall as its tallest item's minimum contribution, so a
// rail whose content outgrows the row would — without the growth cap on the `.toolrail`
// cell — push the row, the grid, the panel host and the canvas past the viewport; the
// document then scrolls, and every raw page coordinate measured before that scroll lands off
// the canvas. Asserted with the rail genuinely overflowing its cell (the GM rail plus the
// place tool's picker), otherwise the no-overflow claim is vacuous.
test("a tool rail taller than the viewport scrolls inside its cell; the grid and canvas never grow past 100vh", async ({
  page,
  account,
}) => {
  const viewport = { width: 1280, height: 720 };
  await page.setViewportSize(viewport);
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Tall Rail World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
  // The place tool adds its asset picker to the rail, on top of the GM's full tool set.
  await page.getByTestId("tool-place").click();
  const rail = page.locator(".toolrail");
  await expect(rail.locator(".asset-picker")).toBeVisible();

  // Positive control: the rail's content really is taller than its cell.
  await expect
    .poll(() => rail.evaluate((el) => el.scrollHeight - el.clientHeight))
    .toBeGreaterThan(0);

  const overflow = () =>
    page.evaluate(() => {
      const layout = document.querySelector(".layout")!;
      return {
        document: document.documentElement.scrollHeight - innerHeight,
        grid: layout.scrollHeight - layout.clientHeight,
        gridHeight: Math.round(layout.getBoundingClientRect().height),
      };
    });
  expect(await overflow()).toEqual({ document: 0, grid: 0, gridHeight: viewport.height });

  const canvas = page.getByTestId("stage-canvas");
  const before = (await canvas.boundingBox())!;
  expect(before.y + before.height).toBeLessThanOrEqual(viewport.height);

  // Reaching a control at the bottom of the rail scrolls the rail cell, never the document,
  // so a canvas rect measured beforehand stays valid for a raw-coordinate gesture.
  await rail.getByTestId("emote-send").scrollIntoViewIfNeeded();
  await expect
    .poll(() => rail.evaluate((el) => el.scrollTop))
    .toBeGreaterThan(0);
  expect(await canvas.boundingBox()).toEqual(before);
  expect(await overflow()).toEqual({ document: 0, grid: 0, gridHeight: viewport.height });
});
