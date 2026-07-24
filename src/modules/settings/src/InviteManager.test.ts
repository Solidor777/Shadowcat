import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import InviteManager from "./InviteManager.svelte";

const CODE = "0123456789abcdef0123456789abcdef.fedcba9876543210fedcba9876543210";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    createWorldInvite: vi.fn().mockResolvedValue({
      id: "i-1",
      code: "0123456789abcdef0123456789abcdef.fedcba9876543210fedcba9876543210",
      role: "player",
      expires_at: Date.now() + 1000,
    }),
    listWorldInvites: vi.fn().mockResolvedValue([]),
    revokeWorldInvite: vi.fn().mockResolvedValue(undefined),
    listWorldMembers: vi.fn().mockResolvedValue([]),
  };
});

describe("InviteManager", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders nothing for a non-GM member", () => {
    const { container } = render(InviteManager, {
      context: setAppContextForTest({ role: "player" }),
    });
    expect(container.querySelector(".invite-manager")).toBeNull();
  });

  it("mints an invite for the chosen world role and shows the code once", async () => {
    const { createWorldInvite } = await import("@shadowcat/core");
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await fireEvent.change(screen.getByLabelText("settings.invites.role"), {
      target: { value: "spectator" },
    });
    await fireEvent.click(screen.getByText("settings.invites.mint"));

    await vi.waitFor(() =>
      expect(vi.mocked(createWorldInvite)).toHaveBeenCalledWith("w1", "spectator"),
    );
    const field = (await screen.findByLabelText("settings.invites.code")) as HTMLInputElement;
    expect(field.value).toBe(CODE);
  });

  it("offers only world-tier roles — no server tier is selectable", () => {
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });
    const select = screen.getByLabelText("settings.invites.role") as HTMLSelectElement;
    const values = [...select.options].map((o) => o.value);
    expect(values).toEqual(["player", "spectator", "gm"]);
    expect(values).not.toContain("admin");
    expect(values).not.toContain("user");
  });

  it("never offers to name an account", () => {
    const { container } = render(InviteManager, {
      context: setAppContextForTest({ role: "gm", world: "w1" }),
    });
    // The surface carries no free-text account field: the whole point of the
    // invite flow is that a GM cannot probe for a username.
    const writable = [...container.querySelectorAll("input")].filter((i) => !i.readOnly);
    expect(writable).toHaveLength(0);
  });

  it("revokes a live invite and re-reads the listing", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.listWorldInvites).mockResolvedValue([
      {
        id: "i-1",
        role: "player",
        created_at: 1,
        expires_at: Date.now() + 10_000,
        revoked_at: null,
        consumed_at: null,
      },
    ]);
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await screen.findByText("settings.invites.revoke");
    const before = vi.mocked(core.listWorldInvites).mock.calls.length;
    await fireEvent.click(screen.getByText("settings.invites.revoke"));
    await vi.waitFor(() =>
      expect(vi.mocked(core.revokeWorldInvite)).toHaveBeenCalledWith("w1", "i-1"),
    );
    await vi.waitFor(() =>
      expect(vi.mocked(core.listWorldInvites).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("keeps the invite list usable when only the roster read fails", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.listWorldInvites).mockResolvedValue([
      {
        id: "i-1",
        role: "player",
        created_at: 1,
        expires_at: Date.now() + 10_000,
        revoked_at: null,
        consumed_at: null,
      },
    ]);
    // The two reads are independent: a failed roster must not take the invite
    // list — and with it the revoke button for a live code — down with it.
    vi.mocked(core.listWorldMembers).mockRejectedValue(new Error("roster down"));
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    expect(await screen.findByText("settings.invites.revoke")).toBeTruthy();
    expect(await screen.findByText("settings.invites.error")).toBeTruthy();
  });

  it("offers no revoke for a spent invite", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.listWorldInvites).mockResolvedValue([
      {
        id: "i-used",
        role: "player",
        created_at: 1,
        expires_at: Date.now() + 10_000,
        revoked_at: null,
        consumed_at: 5,
      },
    ]);
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    expect(await screen.findByText(/settings.invites.consumed/)).toBeTruthy();
    expect(screen.queryByText("settings.invites.revoke")).toBeNull();
  });

  it("re-reads the roster from the server after every change it causes", async () => {
    const core = await import("@shadowcat/core");
    // AppContext's `members` map is a session-start snapshot; a seat added
    // during the session would never appear in it.
    vi.mocked(core.listWorldMembers).mockResolvedValue([
      { user: "u-1", username: "redeemer", role: "player" },
    ]);
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await vi.waitFor(() => expect(vi.mocked(core.listWorldMembers)).toHaveBeenCalledWith("w1"));
    expect(await screen.findByText(/redeemer/)).toBeTruthy();

    // Minting re-reads both lists...
    const before = vi.mocked(core.listWorldMembers).mock.calls.length;
    await fireEvent.click(screen.getByText("settings.invites.mint"));
    await vi.waitFor(() =>
      expect(vi.mocked(core.listWorldMembers).mock.calls.length).toBeGreaterThan(before),
    );

    // ...and so does the explicit refresh, which is how a GM observes a
    // redemption that happened in someone else's session.
    const afterMint = vi.mocked(core.listWorldMembers).mock.calls.length;
    await fireEvent.click(screen.getByText("settings.invites.refresh"));
    await vi.waitFor(() =>
      expect(vi.mocked(core.listWorldMembers).mock.calls.length).toBeGreaterThan(afterMint),
    );
  });

  it("surfaces the server's rejection", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.createWorldInvite).mockRejectedValueOnce(new Error("forbidden"));
    render(InviteManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await fireEvent.click(screen.getByText("settings.invites.mint"));
    expect(await screen.findByText("settings.invites.error")).toBeTruthy();
  });
});
