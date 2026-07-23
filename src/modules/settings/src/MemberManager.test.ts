import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import MemberManager from "./MemberManager.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    addWorldMemberByUsername: vi.fn().mockResolvedValue(undefined),
  };
});

describe("MemberManager", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders nothing for a non-GM member", () => {
    const { container } = render(MemberManager, {
      context: setAppContextForTest({ role: "player" }),
    });
    expect(container.querySelector(".member-manager")).toBeNull();
  });

  it("seats an account by username, trimmed, with the chosen world role", async () => {
    const { addWorldMemberByUsername } = await import("@shadowcat/core");
    render(MemberManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await fireEvent.input(screen.getByLabelText("settings.members.username"), {
      target: { value: "  seated  " },
    });
    await fireEvent.change(screen.getByLabelText("settings.members.role"), {
      target: { value: "spectator" },
    });
    await fireEvent.click(screen.getByText("settings.members.add"));

    await vi.waitFor(() =>
      expect(vi.mocked(addWorldMemberByUsername)).toHaveBeenCalledWith(
        "w1",
        "seated",
        "spectator",
      ),
    );
  });

  it("offers only world-tier roles — no server tier is selectable", () => {
    render(MemberManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });
    const select = screen.getByLabelText("settings.members.role") as HTMLSelectElement;
    const values = [...select.options].map((o) => o.value);
    expect(values).toEqual(["player", "spectator", "gm"]);
    expect(values).not.toContain("admin");
    expect(values).not.toContain("user");
  });

  it("surfaces the server's rejection for an unknown account", async () => {
    const { addWorldMemberByUsername } = await import("@shadowcat/core");
    vi.mocked(addWorldMemberByUsername).mockRejectedValueOnce(new Error("not found"));
    render(MemberManager, { context: setAppContextForTest({ role: "gm", world: "w1" }) });

    await fireEvent.input(screen.getByLabelText("settings.members.username"), {
      target: { value: "no-such-user" },
    });
    await fireEvent.click(screen.getByText("settings.members.add"));

    expect(await screen.findByText("settings.members.error")).toBeTruthy();
  });
});
