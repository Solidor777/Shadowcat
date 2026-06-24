import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import Login from "./Login.svelte";
import * as api from "../api";

afterEach(() => vi.restoreAllMocks());

test("calls onAuthed after a successful login", async () => {
  vi.spyOn(api, "login").mockResolvedValue(true);
  const onAuthed = vi.fn();
  render(Login, { props: { onAuthed } });

  await fireEvent.input(screen.getByLabelText("Username"), { target: { value: "gm" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "pw" } });
  await fireEvent.click(screen.getByRole("button", { name: "Log in" }));

  await waitFor(() => expect(onAuthed).toHaveBeenCalledOnce());
});

test("shows an error and does not call onAuthed on failure", async () => {
  vi.spyOn(api, "login").mockResolvedValue(false);
  const onAuthed = vi.fn();
  render(Login, { props: { onAuthed } });

  await fireEvent.input(screen.getByLabelText("Username"), { target: { value: "gm" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "x" } });
  await fireEvent.click(screen.getByRole("button", { name: "Log in" }));

  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(onAuthed).not.toHaveBeenCalled();
});
