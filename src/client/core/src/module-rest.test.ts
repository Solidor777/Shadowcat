import { expect, test, vi, afterEach } from "vitest";
import { listInstalledModules, getEnabledModules, setEnabledModules } from "./module-rest";

afterEach(() => {
  vi.unstubAllGlobals();
});

test("listInstalledModules GETs /api/modules and returns the parsed array", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => [{ manifest: { id: "a" }, entry_url: "/modules/a/index.js" }],
  });
  vi.stubGlobal("fetch", fetchMock);
  const got = await listInstalledModules();
  expect(fetchMock).toHaveBeenCalledWith("/api/modules", expect.any(Object));
  expect(got).toEqual([{ manifest: { id: "a" }, entry_url: "/modules/a/index.js" }]);
});

test("listInstalledModules throws on a non-ok response", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 401 }));
  await expect(listInstalledModules()).rejects.toThrow(/401/);
});

test("getEnabledModules GETs the world's enabled-modules endpoint", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ["a", "b"] });
  vi.stubGlobal("fetch", fetchMock);
  const got = await getEnabledModules("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1/enabled-modules", expect.any(Object));
  expect(got).toEqual(["a", "b"]);
});

test("setEnabledModules PUTs the ids as a JSON body", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal("fetch", fetchMock);
  await setEnabledModules("w1", ["a", "b"]);
  expect(fetchMock).toHaveBeenCalledWith(
    "/api/worlds/w1/enabled-modules",
    expect.objectContaining({
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(["a", "b"]),
    }),
  );
});

test("setEnabledModules throws on a non-ok response", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 422 }));
  await expect(setEnabledModules("w1", ["a"])).rejects.toThrow(/422/);
});
