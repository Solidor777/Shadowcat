import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssetResolver } from "./assets";
import {
  bulkPatchAssets,
  deleteAsset,
  getAssetMeta,
  listAssets,
  originalUrl,
  patchAsset,
  queryAssets,
  reconvertAsset,
  replaceAsset,
  uploadAsset,
} from "./asset-rest";
import { startChunkedUpload } from "./asset-upload";
import {
  adminBackup,
  deleteBackup,
  dismissRestoreFailed,
  listBackups,
  pruneBackups,
  restoreBackup,
} from "./backup-rest";
import { getEnabledModules, listInstalledModules, setEnabledModules } from "./module-rest";
import { getPortMapStatus, setPortForward } from "./portmap-rest";
import { deleteTrashEntry, emptyTrash, listTrash } from "./trash-rest";
import {
  avatarUrl,
  changeOwnPassword,
  createUser,
  createWorldInvite,
  deleteAvatar,
  deleteUser,
  getUiState,
  listUsers,
  listWorldInvites,
  listWorldMembers,
  revokeWorldInvite,
  updateProfile,
  uploadAvatar,
} from "./user-rest";

/** The global the server's `index.html` substitution writes. */
const BASE_KEY = "__SHADOWCAT_BASE__";

/** The prefix the server is mounted under in these tests. */
const PREFIX = "/ttrpg";

/** A world id used where a call needs one. */
const WORLD = "00000000-0000-0000-0000-000000000001";

/** A request a wrapper issued. */
interface Seen {
  /** The URL the wrapper passed to `fetch`. */
  url: string;
}

/** The URLs every `fetch` call in the current test received, in order. */
let seen: Seen[] = [];

/** A file the upload wrappers accept. */
const file = new File([new Uint8Array([1, 2, 3])], "a.png", { type: "image/png" });

/**
 * Runs `call` against a `fetch` that refuses every request, so the wrapper throws after it has
 * built its URL. Only the URL is under test.
 *
 * @param call The wrapper invocation.
 * @returns The first URL the wrapper requested.
 */
async function firstUrl(call: () => unknown): Promise<string> {
  seen = [];
  await Promise.resolve()
    .then(call)
    .catch(() => undefined);
  expect(seen.length).toBeGreaterThan(0);
  return seen[0].url;
}

describe("every same-origin REST wrapper applies the route prefix", () => {
  beforeEach(() => {
    (globalThis as Record<string, unknown>)[BASE_KEY] = PREFIX;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: unknown) => {
        seen.push({ url: String(input) });
        return new Response("", { status: 500 });
      }),
    );
  });

  afterEach(() => {
    delete (globalThis as Record<string, unknown>)[BASE_KEY];
    vi.unstubAllGlobals();
  });

  const calls: Array<[string, () => unknown]> = [
    ["uploadAsset", () => uploadAsset(WORLD, file)],
    ["listAssets", () => listAssets(WORLD)],
    ["getAssetMeta", () => getAssetMeta("a1")],
    ["replaceAsset", () => replaceAsset("a1", file)],
    ["deleteAsset", () => deleteAsset("a1")],
    ["queryAssets", () => queryAssets(WORLD, {})],
    ["patchAsset", () => patchAsset("a1", {})],
    ["bulkPatchAssets", () => bulkPatchAssets(WORLD, { ids: [], action: "delete" } as never)],
    ["reconvertAsset", () => reconvertAsset("a1")],
    ["startChunkedUpload", () => startChunkedUpload(WORLD, file)],
    ["adminBackup", () => adminBackup()],
    ["listBackups", () => listBackups()],
    ["deleteBackup", () => deleteBackup("b1")],
    ["restoreBackup", () => restoreBackup("b1")],
    ["dismissRestoreFailed", () => dismissRestoreFailed()],
    ["pruneBackups", () => pruneBackups()],
    ["listInstalledModules", () => listInstalledModules()],
    ["getEnabledModules", () => getEnabledModules(WORLD)],
    ["setEnabledModules", () => setEnabledModules(WORLD, [])],
    ["getPortMapStatus", () => getPortMapStatus()],
    ["setPortForward", () => setPortForward(true)],
    ["listTrash", () => listTrash()],
    ["emptyTrash", () => emptyTrash()],
    ["deleteTrashEntry", () => deleteTrashEntry("t1")],
    ["listUsers", () => listUsers()],
    ["listWorldMembers", () => listWorldMembers(WORLD)],
    ["updateProfile", () => updateProfile({} as never)],
    ["uploadAvatar", () => uploadAvatar(new Blob([]))],
    ["deleteAvatar", () => deleteAvatar()],
    ["createUser", () => createUser({} as never)],
    ["deleteUser", () => deleteUser("u1")],
    ["changeOwnPassword", () => changeOwnPassword("a", "b")],
    ["createWorldInvite", () => createWorldInvite(WORLD, "player")],
    ["listWorldInvites", () => listWorldInvites(WORLD)],
    ["revokeWorldInvite", () => revokeWorldInvite(WORLD, "c1")],
    ["getUiState", () => getUiState()],
  ];

  it.each(calls)("%s requests a prefixed /api path", async (_name, call) => {
    expect(await firstUrl(call)).toMatch(new RegExp(`^${PREFIX}/api/`));
  });

  it("builds prefixed asset, original and avatar URLs", () => {
    const resolver = new AssetResolver();
    expect(resolver.url("a1")).toBe(`${PREFIX}/api/assets/a1`);
    expect(resolver.url("a1", "thumb")).toBe(`${PREFIX}/api/assets/a1?variant=thumb`);
    expect(originalUrl("a1")).toBe(`${PREFIX}/api/assets/a1/original`);
    expect(avatarUrl(WORLD, "u1", true)).toBe(`${PREFIX}/api/worlds/${WORLD}/members/u1/avatar`);
  });

  it("leaves every URL unprefixed at the root mount", async () => {
    delete (globalThis as Record<string, unknown>)[BASE_KEY];
    expect(await firstUrl(() => listUsers())).toBe("/api/users");
    expect(new AssetResolver().url("a1")).toBe("/api/assets/a1");
  });
});
