import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import WorldSelect from "./WorldSelect.svelte";
import * as api from "../api";

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
