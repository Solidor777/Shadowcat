import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ChangePasswordForm from "./ChangePasswordForm.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return { ...actual, changeOwnPassword: vi.fn().mockResolvedValue(undefined) };
});

/** Fills the three password fields and submits the form. */
async function fill(current: string, next: string, confirm: string): Promise<void> {
  await fireEvent.input(screen.getByLabelText("settings.account.current"), {
    target: { value: current },
  });
  await fireEvent.input(screen.getByLabelText("settings.account.new"), {
    target: { value: next },
  });
  await fireEvent.input(screen.getByLabelText("settings.account.confirm"), {
    target: { value: confirm },
  });
  await fireEvent.click(screen.getByText("settings.account.submit"));
}

describe("ChangePasswordForm", () => {
  beforeEach(() => vi.clearAllMocks());

  it("blocks the request when the confirmation does not match", async () => {
    const { changeOwnPassword } = await import("@shadowcat/core");
    const logout = vi.fn(async () => {});
    render(ChangePasswordForm, { context: setAppContextForTest({ logout }) });
    await fill("old-pw", "new-password", "different-password");
    expect(await screen.findByText("settings.account.mismatch")).toBeTruthy();
    expect(vi.mocked(changeOwnPassword)).not.toHaveBeenCalled();
    expect(logout).not.toHaveBeenCalled();
  });

  it("sends both passwords, then ends the local session on success", async () => {
    const { changeOwnPassword } = await import("@shadowcat/core");
    const logout = vi.fn(async () => {});
    render(ChangePasswordForm, { context: setAppContextForTest({ logout }) });
    await fill("old-pw", "new-password", "new-password");
    await vi.waitFor(() => expect(logout).toHaveBeenCalledOnce());
    expect(vi.mocked(changeOwnPassword)).toHaveBeenCalledWith("old-pw", "new-password");
  });

  it("shows the fixed incorrect-current-password text on a 401 and keeps the session", async () => {
    const { changeOwnPassword, ChangePasswordError } = await import("@shadowcat/core");
    vi.mocked(changeOwnPassword).mockRejectedValueOnce(new ChangePasswordError("unauthorized", 401));
    const logout = vi.fn(async () => {});
    render(ChangePasswordForm, { context: setAppContextForTest({ logout }) });
    await fill("wrong", "new-password", "new-password");
    expect(await screen.findByText("settings.account.currentIncorrect")).toBeTruthy();
    expect(logout).not.toHaveBeenCalled();
  });

  it("shows the server's message for any other refusal, such as a policy failure", async () => {
    const { changeOwnPassword, ChangePasswordError } = await import("@shadowcat/core");
    vi.mocked(changeOwnPassword).mockRejectedValueOnce(
      new ChangePasswordError("password too short", 422),
    );
    const logout = vi.fn(async () => {});
    render(ChangePasswordForm, { context: setAppContextForTest({ logout }) });
    await fill("old-pw", "short", "short");
    expect(await screen.findByText("settings.account.failed")).toBeTruthy();
    expect(logout).not.toHaveBeenCalled();
  });
});
