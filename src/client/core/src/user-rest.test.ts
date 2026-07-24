import { expect, test, vi, afterEach } from "vitest";
import {
  listUsers,
  createUser,
  listWorldMembers,
  createWorldInvite,
  listWorldInvites,
  revokeWorldInvite,
} from "./user-rest";

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

test("listUsers surfaces the server's reason on the non-admin 403", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({ ok: false, status: 403, json: async () => ({ error: "forbidden" }) }),
  );
  await expect(listUsers()).rejects.toThrow(/forbidden/);
});

test("listUsers falls back to the status when the body is not JSON", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => {
        throw new Error("not json");
      },
    }),
  );
  await expect(listUsers()).rejects.toThrow(/403/);
});

test("listWorldMembers GETs the world roster", async () => {
  const member = { user: "u-1", username: "player-one", role: "player" };
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => [member] });
  vi.stubGlobal("fetch", fetchMock);
  const got = await listWorldMembers("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1/members", expect.any(Object));
  expect(got).toEqual([member]);
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

test("createWorldInvite POSTs only a world role and returns the minted code", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({
      id: "i-1",
      code: "0123456789abcdef0123456789abcdef.fedcba9876543210fedcba9876543210",
      role: "player",
      expires_at: 1000,
    }),
  });
  vi.stubGlobal("fetch", fetchMock);
  const got = await createWorldInvite("w1", "player");

  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  expect(url).toBe("/api/worlds/w1/invites");
  expect(init.method).toBe("POST");
  // No account is named and no server-tier field is sent: the request carries
  // a WorldRole and nothing else.
  expect(JSON.parse(init.body as string)).toEqual({ role: "player" });
  expect(got.code).toContain(".");
});

test("createWorldInvite surfaces the server's rejection reason", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => ({ error: "forbidden" }),
    }),
  );
  await expect(createWorldInvite("w1", "player")).rejects.toThrow(/forbidden/);
});

test("listWorldInvites GETs the world's invites", async () => {
  const entry = {
    id: "i-1",
    role: "player",
    created_at: 1,
    expires_at: 2,
    revoked_at: null,
    consumed_at: null,
  };
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => [entry] });
  vi.stubGlobal("fetch", fetchMock);
  const got = await listWorldInvites("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1/invites", expect.any(Object));
  expect(got).toEqual([entry]);
});

test("revokeWorldInvite DELETEs the invite by id", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal("fetch", fetchMock);
  await revokeWorldInvite("w1", "i-1");
  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  expect(url).toBe("/api/worlds/w1/invites/i-1");
  expect(init.method).toBe("DELETE");
});

test("revokeWorldInvite surfaces a rejection", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({ ok: false, status: 404, json: async () => ({ error: "not found" }) }),
  );
  await expect(revokeWorldInvite("w1", "i-ghost")).rejects.toThrow(/not found/);
});
