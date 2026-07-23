import { expect, test, vi, afterEach } from "vitest";
import { listUsers, createUser, addWorldMemberByUsername } from "./user-rest";

afterEach(() => {
  vi.unstubAllGlobals();
});

test("listUsers GETs /api/users and returns the parsed accounts", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => [{ id: "u-1", username: "root-admin", server_role: "admin" }],
  });
  vi.stubGlobal("fetch", fetchMock);
  const got = await listUsers();
  expect(fetchMock).toHaveBeenCalledWith("/api/users", expect.any(Object));
  expect(got).toEqual([{ id: "u-1", username: "root-admin", server_role: "admin" }]);
});

test("listUsers throws on the non-admin 403", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 403 }));
  await expect(listUsers()).rejects.toThrow(/403/);
});

test("createUser POSTs the credential once and omits server_role when unset", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({ id: "u-3", username: "new-player", server_role: "user" }),
  });
  vi.stubGlobal("fetch", fetchMock);
  const got = await createUser({ username: "new-player", password: "pw-new-player" });

  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  expect(url).toBe("/api/users");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body as string)).toEqual({
    username: "new-player",
    password: "pw-new-player",
  });
  expect(got.server_role).toBe("user");
});

test("createUser forwards an explicit admin tier", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({ id: "u-4", username: "second-admin", server_role: "admin" }),
  });
  vi.stubGlobal("fetch", fetchMock);
  await createUser({ username: "second-admin", password: "pw-second", serverRole: "admin" });
  const init = fetchMock.mock.calls[0][1] as RequestInit;
  expect(JSON.parse(init.body as string).server_role).toBe("admin");
});

test("createUser surfaces the server's rejection reason, not a bare status", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: false,
      status: 409,
      json: async () => ({ error: "username already taken" }),
    }),
  );
  await expect(
    createUser({ username: "root-admin", password: "pw-collision" }),
  ).rejects.toThrow(/username already taken/);
});

test("createUser falls back to the status when the body is not JSON", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: false,
      status: 500,
      json: async () => {
        throw new Error("not json");
      },
    }),
  );
  await expect(createUser({ username: "x-user", password: "pw-x" })).rejects.toThrow(/500/);
});

test("addWorldMemberByUsername POSTs a username + world role to the members route", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
  vi.stubGlobal("fetch", fetchMock);
  await addWorldMemberByUsername("w1", "seated", "player");

  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  expect(url).toBe("/api/worlds/w1/members");
  expect(init.method).toBe("POST");
  // No `user` id and no server-tier field: the GM path carries a name and a
  // WorldRole, nothing else.
  expect(JSON.parse(init.body as string)).toEqual({ username: "seated", role: "player" });
});

test("addWorldMemberByUsername surfaces an unknown-account rejection", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({ ok: false, status: 404, json: async () => ({ error: "not found" }) }),
  );
  await expect(addWorldMemberByUsername("w1", "ghost-user", "player")).rejects.toThrow(
    /not found/,
  );
});
