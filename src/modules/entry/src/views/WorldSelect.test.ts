import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import WorldSelect from "./WorldSelect.svelte";
import * as api from "../entryApi";

afterEach(() => vi.restoreAllMocks());

test("lists worlds and enters the chosen one", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "w1", name: "Alpha", role: "gm" },
    { id: "w2", name: "Beta", role: "player" },
  ]);
  const onEnter = vi.fn();
  render(WorldSelect, { props: { onEnter } });

  await waitFor(() => expect(screen.getByText("Alpha")).toBeTruthy());
  await fireEvent.click(screen.getByRole("button", { name: /Alpha/ }));
  expect(onEnter).toHaveBeenCalledWith("w1");
});

test("delete affordance is gm-only and arms on exact name", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "w1", name: "Alpha", role: "gm" },
    { id: "w2", name: "Beta", role: "player" },
  ]);
  const del = vi.spyOn(api, "deleteWorld").mockResolvedValue(undefined);
  render(WorldSelect, { props: { onEnter: vi.fn() } });

  expect(await screen.findAllByRole("button", { name: "Delete" })).toHaveLength(1);
  await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  const confirm = screen.getByRole("button", { name: "Delete forever" }) as HTMLButtonElement;
  expect(confirm.disabled).toBe(true);
  await fireEvent.input(screen.getByLabelText(/Type the world name/), {
    target: { value: "Alph" },
  });
  expect(confirm.disabled).toBe(true);
  await fireEvent.input(screen.getByLabelText(/Type the world name/), {
    target: { value: "Alpha" },
  });
  expect(confirm.disabled).toBe(false);
  await fireEvent.click(confirm);
  expect(del).toHaveBeenCalledWith("w1");
});

test("delete failure shows the delete error", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "w1", name: "Alpha", role: "gm" },
  ]);
  vi.spyOn(api, "deleteWorld").mockRejectedValue(new Error("403"));
  render(WorldSelect, { props: { onEnter: vi.fn() } });

  await fireEvent.click(
    await screen.findByRole("button", { name: "Delete" }),
  );
  await fireEvent.input(screen.getByLabelText(/Type the world name/), {
    target: { value: "Alpha" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Delete forever" }));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe("Could not delete the world."),
  );
});

test("redeeming an invite enters the world it seats the caller in", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  const accept = vi
    .spyOn(api, "acceptInvite")
    .mockResolvedValue({ id: "w9", name: "Invited", role: "player" });
  const onEnter = vi.fn();
  render(WorldSelect, { props: { onEnter } });

  await fireEvent.input(screen.getByLabelText("Invite code"), {
    target: { value: "  code-abc  " },
  });
  await fireEvent.click(screen.getByText("Join with an invite code"));

  await waitFor(() => expect(accept).toHaveBeenCalledWith("code-abc"));
  await waitFor(() => expect(onEnter).toHaveBeenCalledWith("w9"));
});

test("an unusable invite reports one generic failure and enters nothing", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  // The server answers unknown / expired / revoked / already-used identically;
  // the view must not try to explain which.
  vi.spyOn(api, "acceptInvite").mockResolvedValue(null);
  const onEnter = vi.fn();
  render(WorldSelect, { props: { onEnter } });

  await fireEvent.input(screen.getByLabelText("Invite code"), {
    target: { value: "no-such-code" },
  });
  await fireEvent.click(screen.getByText("Join with an invite code"));

  await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("That invite code is not usable."));
  expect(onEnter).not.toHaveBeenCalled();
});
