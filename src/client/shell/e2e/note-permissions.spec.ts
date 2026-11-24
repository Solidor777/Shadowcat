import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

// The note sheet's `PermissionsEditor`: the GM sets a note's default role and a per-user override,
// and a seated player's notes panel follows live. A private note is invisible to the player; a
// per-user "Can view" override reveals it to that player alone while the default stays "No
// access"; removing the override hides it again; and a default of "Can view" reveals it with no
// override at all.
test("note permissions: a per-user override and the default role each reveal a note to a player", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Note Permissions World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openPanel(gm, "settings:panel");
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await newE2EContext(browser, { baseURL: test.info().project.use.baseURL });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The member roster is a session-start snapshot: re-enter so the picker lists the player.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await openPanel(gm, "notes:panel");
    await gm.getByTestId("notes-name").fill("Secret plans");
    await gm.getByTestId("notes-create").click();
    const sheet = gm.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: "Secret plans" });
    await expect(sheet).toBeVisible({ timeout: 15_000 });

    await openPanel(player, "notes:panel");
    await expect(player.getByTestId("note-row")).toHaveCount(0);

    const editor = sheet.getByTestId("permissions-editor");
    await expect(editor).toBeVisible();
    await expect(editor.getByTestId("permissions-default")).toHaveValue("none");

    // A per-user override: the player is granted "Can view" while the default stays "No access".
    // The creator already holds an override row of their own, so the player's row is found by name.
    const override = editor
      .locator("label", { has: gm.getByTestId("permissions-user-role") })
      .filter({ hasText: playerName })
      .getByTestId("permissions-user-role");
    await expect(override).toHaveCount(0);
    await editor.getByRole("checkbox", { name: playerName }).check();
    await expect(override).toHaveCount(1);
    await expect(override).toHaveValue("observer");
    await expect(player.getByTestId("note-row")).toHaveCount(1, { timeout: 15_000 });
    await expect(editor.getByTestId("permissions-default")).toHaveValue("none");

    // The player opens the note as a reader: no edit control, no permissions editor.
    await player.getByTestId("note-open").click();
    const playerSheet = player
      .getByRole("dialog", { name: "Sheet", exact: true })
      .filter({ hasText: "Secret plans" });
    await expect(playerSheet).toBeVisible({ timeout: 15_000 });
    await expect(playerSheet.getByTestId("permissions-editor")).toHaveCount(0);
    await expect(playerSheet.getByTestId("note-edit")).toHaveCount(0);

    // Removing the override hides the note again.
    await editor.getByRole("checkbox", { name: playerName }).uncheck();
    await expect(override).toHaveCount(0);
    await expect(player.getByTestId("note-row")).toHaveCount(0, { timeout: 15_000 });

    // The default role: "Can view" reveals the note with no override.
    await editor.getByTestId("permissions-default").selectOption("observer");
    await expect(player.getByTestId("note-row")).toHaveCount(1, { timeout: 15_000 });
    await expect(override).toHaveCount(0);
  } finally {
    await playerCtx.close();
  }
});
