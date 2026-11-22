import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { getPortMapStatus, setPortForward } from "./portmap-rest";

/** A fetch response with a JSON body. */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** A status body as the server's JSON carries it. */
const STATUS = {
  enabled: true,
  locked_by_config: false,
  state: "mapped",
  protocol: "nat_pmp",
  external_address: "203.0.113.7",
  external_port: 30000,
  lease_expires_in_secs: 1800,
  reason: null,
  detail: null,
};

describe("portmap-rest", () => {
  const fetchMock = vi.fn();
  beforeEach(() => {
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("getPortMapStatus GETs the admin route and returns the status", async () => {
    fetchMock.mockResolvedValue(json(200, STATUS));
    expect(await getPortMapStatus()).toEqual(STATUS);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/portmap");
  });

  it("getPortMapStatus surfaces the server's error text", async () => {
    fetchMock.mockResolvedValue(json(403, { error: "forbidden" }));
    await expect(getPortMapStatus()).rejects.toThrow("forbidden");
  });

  it("setPortForward PUTs the choice and returns the applied status", async () => {
    fetchMock.mockResolvedValue(json(200, STATUS));
    expect(await setPortForward(true)).toEqual(STATUS);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/admin/portmap");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body)).toEqual({ enabled: true });
  });

  it("setPortForward rejects with the server's text when the config locks the toggle", async () => {
    fetchMock.mockResolvedValue(json(409, { error: "locked by port_forward" }));
    await expect(setPortForward(false)).rejects.toThrow("locked by port_forward");
  });
});
