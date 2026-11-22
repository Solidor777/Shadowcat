import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import PortForwardSection from "./PortForwardSection.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return { ...actual, getPortMapStatus: vi.fn(), setPortForward: vi.fn() };
});

/** A status body, overridable per test. */
function status(over: Record<string, unknown> = {}) {
  return {
    enabled: false,
    locked_by_config: false,
    state: "disabled",
    protocol: null,
    external_address: null,
    external_port: null,
    lease_expires_in_secs: null,
    reason: null,
    detail: null,
    ...over,
  };
}

describe("PortForwardSection", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const core = await import("@shadowcat/core");
    vi.mocked(core.getPortMapStatus).mockResolvedValue(status() as never);
    vi.mocked(core.setPortForward).mockResolvedValue(status({ enabled: true, state: "discovering" }) as never);
  });

  it("renders nothing and fetches nothing for a non-admin, even a world GM", async () => {
    const { getPortMapStatus } = await import("@shadowcat/core");
    const { container } = render(PortForwardSection, {
      context: setAppContextForTest({ role: "gm", serverRole: "user" }),
    });
    expect(container.querySelector(".port-forward")).toBeNull();
    expect(vi.mocked(getPortMapStatus)).not.toHaveBeenCalled();
  });

  it("shows the toggle off, the internet-exposure warning and the off state", async () => {
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    const box = (await screen.findByRole("checkbox")) as HTMLInputElement;
    expect(box.checked).toBe(false);
    expect(box.disabled).toBe(false);
    expect(screen.getByText("settings.portForward.warning")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("settings.portForward.state.disabled");
  });

  it("turning the toggle on sends enabled true and shows the returned status", async () => {
    const { setPortForward } = await import("@shadowcat/core");
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    await fireEvent.click(await screen.findByRole("checkbox"));
    expect(vi.mocked(setPortForward)).toHaveBeenCalledWith(true);
    expect((await screen.findByRole("status")).textContent).toContain(
      "settings.portForward.state.discovering",
    );
  });

  it("disables the toggle and says why when the configuration locks it", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.getPortMapStatus).mockResolvedValue(
      status({ enabled: true, locked_by_config: true, state: "discovering" }) as never,
    );
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    const box = (await screen.findByRole("checkbox")) as HTMLInputElement;
    expect(box.disabled).toBe(true);
    expect(screen.getByText("settings.portForward.locked")).toBeTruthy();
  });

  it("shows protocol, external address with port and lease for a mapping", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.getPortMapStatus).mockResolvedValue(
      status({
        enabled: true,
        state: "mapped",
        protocol: "upnp",
        external_address: "203.0.113.7",
        external_port: 30000,
        lease_expires_in_secs: 1800,
      }) as never,
    );
    const { container } = render(PortForwardSection, {
      context: setAppContextForTest({ serverRole: "admin" }),
    });
    await screen.findByRole("status");
    expect(container.textContent).toContain("settings.portForward.protocol.upnp");
    expect(container.textContent).toContain("settings.portForward.address");
    expect(container.textContent).toContain("settings.portForward.lease");
  });

  it("shows the server's detail sentence for a failure", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.getPortMapStatus).mockResolvedValue(
      status({ enabled: true, state: "failed", reason: "no_gateway", detail: "No router answered." }) as never,
    );
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    expect(await screen.findByText("No router answered.")).toBeTruthy();
  });

  it("reports a failed toggle and keeps the prior status", async () => {
    const core = await import("@shadowcat/core");
    vi.mocked(core.setPortForward).mockRejectedValue(new Error("locked"));
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    await fireEvent.click(await screen.findByRole("checkbox"));
    expect((await screen.findByRole("alert")).textContent).toContain("settings.portForward.setError");
    expect(screen.getByRole("status").textContent).toContain("settings.portForward.state.disabled");
  });

  it("Refresh re-reads the status", async () => {
    const { getPortMapStatus } = await import("@shadowcat/core");
    render(PortForwardSection, { context: setAppContextForTest({ serverRole: "admin" }) });
    await fireEvent.click(await screen.findByRole("button"));
    expect(vi.mocked(getPortMapStatus)).toHaveBeenCalledTimes(2);
  });
});
