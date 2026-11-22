import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { SvelteMap } from "svelte/reactivity";
import type { MemberProfile } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ProfileEditor from "./ProfileEditor.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    updateProfile: vi.fn().mockResolvedValue(undefined),
    uploadAvatar: vi.fn().mockResolvedValue(undefined),
    deleteAvatar: vi.fn().mockResolvedValue(undefined),
  };
});

/** Builds a context whose own profile is `own`, returning the reactive map the editor writes to. */
function setup(own?: MemberProfile) {
  const memberProfiles = new SvelteMap<string, MemberProfile>();
  if (own) memberProfiles.set("u-self", own);
  const context = setAppContextForTest({ selfId: "u-self", world: "w1", memberProfiles });
  render(ProfileEditor, { context });
  return memberProfiles;
}

describe("ProfileEditor", () => {
  beforeEach(() => vi.clearAllMocks());

  it("saves colour and pronouns together and shows them on this client's roster", async () => {
    const { updateProfile } = await import("@shadowcat/core");
    const profiles = setup();
    await fireEvent.input(screen.getByLabelText("settings.profile.colour"), {
      target: { value: "#336699" },
    });
    await fireEvent.input(screen.getByLabelText("settings.profile.pronouns"), {
      target: { value: "  they/them " },
    });
    await fireEvent.click(screen.getByText("settings.profile.save"));
    await vi.waitFor(() =>
      expect(vi.mocked(updateProfile)).toHaveBeenCalledWith({ colour: "#336699", pronouns: "they/them" }),
    );
    await vi.waitFor(() =>
      expect(profiles.get("u-self")).toEqual({ colour: "#336699", pronouns: "they/them", avatar: null }),
    );
  });

  it("starts from the stored profile and clears the colour with the clear button", async () => {
    const { updateProfile } = await import("@shadowcat/core");
    setup({ colour: "#112233", pronouns: "she/her", avatar: null });
    expect((screen.getByLabelText("settings.profile.pronouns") as HTMLInputElement).value).toBe("she/her");
    await fireEvent.click(screen.getByText("settings.profile.colourClear"));
    await fireEvent.click(screen.getByText("settings.profile.save"));
    await vi.waitFor(() =>
      expect(vi.mocked(updateProfile)).toHaveBeenCalledWith({ colour: null, pronouns: "she/her" }),
    );
  });

  it("shows the server's message when the save is refused and leaves the roster alone", async () => {
    const { updateProfile } = await import("@shadowcat/core");
    vi.mocked(updateProfile).mockRejectedValueOnce(new Error("bad colour"));
    const profiles = setup();
    await fireEvent.click(screen.getByText("settings.profile.save"));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(profiles.has("u-self")).toBe(false);
  });

  it("uploads a chosen image and previews it, then removes it", async () => {
    const { uploadAvatar, deleteAvatar } = await import("@shadowcat/core");
    const profiles = setup();
    const file = new File([new Uint8Array([1, 2, 3])], "me.png", { type: "image/png" });
    await fireEvent.change(screen.getByLabelText("settings.profile.avatar"), {
      target: { files: [file] },
    });
    await vi.waitFor(() => expect(vi.mocked(uploadAvatar)).toHaveBeenCalledWith(file));
    const preview = (await screen.findByAltText("settings.profile.avatarAlt")) as HTMLImageElement;
    expect(preview.getAttribute("src")).toMatch(/^\/api\/worlds\/w1\/members\/u-self\/avatar\?v=\d+$/);

    await fireEvent.click(screen.getByText("settings.profile.avatarRemove"));
    await vi.waitFor(() => expect(vi.mocked(deleteAvatar)).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(profiles.get("u-self")?.avatar).toBeNull());
    expect(screen.queryByAltText("settings.profile.avatarAlt")).toBeNull();
  });

  it("shows the server's message when the upload is refused and keeps the old avatar", async () => {
    const { uploadAvatar } = await import("@shadowcat/core");
    vi.mocked(uploadAvatar).mockRejectedValueOnce(new Error("too large"));
    const profiles = setup({ colour: null, pronouns: null, avatar: "/old.png" });
    const file = new File([new Uint8Array([1])], "big.png", { type: "image/png" });
    await fireEvent.change(screen.getByLabelText("settings.profile.avatar"), {
      target: { files: [file] },
    });
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(profiles.get("u-self")?.avatar).toBe("/old.png");
  });
});
