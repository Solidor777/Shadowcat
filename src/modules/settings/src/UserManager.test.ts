import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import UserManager from "./UserManager.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listUsers: vi.fn().mockResolvedValue([
      { id: "u-1", username: "root-admin", server_role: "admin" },
      { id: "u-2", username: "plain-player", server_role: "user" },
    ]),
    createUser: vi
      .fn()
      .mockResolvedValue({ id: "u-3", username: "new-player", server_role: "user" }),
  };
});

describe("UserManager", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders nothing and fetches nothing for a non-admin, even a world GM", async () => {
    const { listUsers } = await import("@shadowcat/core");
    const { container } = render(UserManager, {
      context: setAppContextForTest({ role: "gm", serverRole: "user" }),
    });
    expect(container.querySelector(".user-manager")).toBeNull();
    expect(vi.mocked(listUsers)).not.toHaveBeenCalled();
  });

  it("lists accounts and creates one for an admin, sending the role and never echoing the password", async () => {
    const { createUser } = await import("@shadowcat/core");
    render(UserManager, {
      context: setAppContextForTest({ role: "player", serverRole: "admin" }),
    });

    expect(await screen.findByText("root-admin")).toBeTruthy();
    expect(screen.getByText("plain-player")).toBeTruthy();

    await fireEvent.input(screen.getByLabelText("settings.users.username"), {
      target: { value: "new-player" },
    });
    const pw = screen.getByLabelText("settings.users.password") as HTMLInputElement;
    // The plaintext is masked in the DOM.
    expect(pw.type).toBe("password");
    await fireEvent.input(pw, { target: { value: "pw-new-player" } });
    await fireEvent.click(screen.getByText("settings.users.create"));

    await vi.waitFor(() =>
      expect(vi.mocked(createUser)).toHaveBeenCalledWith({
        username: "new-player",
        password: "pw-new-player",
        serverRole: "user",
      }),
    );
    // Both fields are cleared once the request resolves.
    await vi.waitFor(() => expect(pw.value).toBe(""));
  });

  it("mints an admin when the administrator checkbox is set", async () => {
    const { createUser } = await import("@shadowcat/core");
    render(UserManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    await screen.findByText("root-admin");

    await fireEvent.input(screen.getByLabelText("settings.users.username"), {
      target: { value: "second-admin" },
    });
    await fireEvent.input(screen.getByLabelText("settings.users.password"), {
      target: { value: "pw-second-admin" },
    });
    await fireEvent.click(screen.getByLabelText("settings.users.makeAdmin"));
    await fireEvent.click(screen.getByText("settings.users.create"));

    await vi.waitFor(() =>
      expect(vi.mocked(createUser)).toHaveBeenCalledWith(
        expect.objectContaining({ serverRole: "admin" }),
      ),
    );
  });

  it("surfaces the server's rejection instead of silently failing", async () => {
    const { createUser } = await import("@shadowcat/core");
    vi.mocked(createUser).mockRejectedValueOnce(new Error("username already taken"));
    render(UserManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    await screen.findByText("root-admin");

    await fireEvent.input(screen.getByLabelText("settings.users.username"), {
      target: { value: "root-admin" },
    });
    await fireEvent.input(screen.getByLabelText("settings.users.password"), {
      target: { value: "pw-collision" },
    });
    await fireEvent.click(screen.getByText("settings.users.create"));

    expect(await screen.findByText("settings.users.error")).toBeTruthy();
  });
});
