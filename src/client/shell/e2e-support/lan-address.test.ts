// @vitest-environment node
import { describe, expect, it } from "vitest";
import type { NetworkInterfaceInfo } from "node:os";
import type { TestInfo } from "@playwright/test";
import { classifyAddress, guardPrivateAddress, pickLanAddress } from "./lan-address";

// Adapter names and addresses are synthetic: names are generic tokens the virtual-adapter filter
// keys on, addresses come from the documentation ranges of RFC 5737 (192.0.2.0/24,
// 198.51.100.0/24, 203.0.113.0/24) and RFC 3849 (2001:db8::/32).

function v4(address: string, internal = false): NetworkInterfaceInfo {
  return { address, family: "IPv4", internal } as NetworkInterfaceInfo;
}

describe("pickLanAddress", () => {
  it("uses the override verbatim", () => {
    expect(pickLanAddress({ phys0: [v4("192.0.2.20")] }, "198.51.100.9")).toBe("198.51.100.9");
  });

  it("skips loopback and virtual adapters in favour of a physical one", () => {
    const ifs = {
      lo: [v4("127.0.0.1", true)],
      "tun-test0": [v4("198.51.100.2")],
      "docker-test0": [v4("203.0.113.1")],
      phys0: [v4("192.0.2.160")],
    };
    expect(pickLanAddress(ifs, undefined)).toBe("192.0.2.160");
  });

  it("falls back to a virtual adapter when it is the only one", () => {
    expect(pickLanAddress({ "docker-test0": [v4("203.0.113.1")] }, undefined)).toBe("203.0.113.1");
  });

  it("skips link-local addresses, which no peer can reach", () => {
    const ifs = { phys0: [v4("169.254.10.20")], phys1: [v4("192.0.2.31")] };
    expect(pickLanAddress(ifs, undefined)).toBe("192.0.2.31");
    expect(() => pickLanAddress({ phys0: [v4("169.254.10.20")] }, undefined)).toThrow(/SHADOWCAT_E2E_LAN_IP/);
  });

  it("ignores IPv6 and internal addresses and throws when nothing remains", () => {
    const ifs = {
      lo: [v4("127.0.0.1", true)],
      phys0: [{ address: "2001:db8::1", family: "IPv6", internal: false } as NetworkInterfaceInfo],
    };
    expect(() => pickLanAddress(ifs, undefined)).toThrow(/SHADOWCAT_E2E_LAN_IP/);
  });
});

describe("classifyAddress", () => {
  it.each([
    ["10.0.0.1", "private"],
    ["10.255.255.254", "private"],
    ["172.16.0.1", "private"],
    ["172.31.255.1", "private"],
    ["172.32.0.1", "public"],
    ["172.15.255.1", "public"],
    ["192.168.0.1", "private"],
    ["100.64.0.1", "cgnat"],
    ["100.127.255.254", "cgnat"],
    ["100.128.0.1", "public"],
    ["100.63.255.255", "public"],
    ["169.254.1.1", "link-local"],
    ["127.0.0.1", "loopback"],
    ["192.0.2.1", "public"],
    ["203.0.113.7", "public"],
    ["not-an-address", "public"],
  ])("%s is %s", (address, expected) => {
    expect(classifyAddress(address)).toBe(expected);
  });
});

describe("guardPrivateAddress", () => {
  /** A `TestInfo` carrying only the annotations list the guard writes to. */
  function stubInfo(): TestInfo & { annotations: { type: string; description?: string }[] } {
    return { annotations: [] } as unknown as TestInfo & { annotations: { type: string; description?: string }[] };
  }

  /** Records every call the guard makes to its skip function. */
  function recorder(): { calls: [boolean, string][]; skip: (c: boolean, r: string) => void } {
    const calls: [boolean, string][] = [];
    return { calls, skip: (c, r) => void calls.push([c, r]) };
  }

  it("lets a private address run: skip is called with false and nothing throws", () => {
    const info = stubInfo();
    const { calls, skip } = recorder();
    guardPrivateAddress("http://10.1.2.3:32001", info, skip, true);
    expect(calls).toEqual([[false, ""]]);
    expect(info.annotations).toEqual([{ type: "address-class", description: "private" }]);
  });

  it("skips a non-private address locally with the verdict's reason and does not throw", () => {
    const { calls, skip } = recorder();
    guardPrivateAddress("http://100.64.0.9:32001", stubInfo(), skip, false);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.[0]).toBe(true);
    expect(calls[0]?.[1]).toContain("cgnat");
  });

  it("throws on CI for a non-private address and never reaches skip", () => {
    const { calls, skip } = recorder();
    expect(() => guardPrivateAddress("http://100.64.0.9:32001", stubInfo(), skip, true)).toThrow(/cgnat/);
    expect(calls).toEqual([]);
  });

  it("does not classify a host name: no skip call and no annotation", () => {
    const info = stubInfo();
    const { calls, skip } = recorder();
    guardPrivateAddress("http://shadowcat.example:32001", info, skip, true);
    expect(calls).toEqual([]);
    expect(info.annotations).toEqual([]);
  });
});
