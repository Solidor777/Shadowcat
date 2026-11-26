import type { TestInfo } from "@playwright/test";
import type { NetworkInterfaceInfo } from "node:os";

/** Adapter names that carry no reachable LAN address: hypervisor switches, container bridges,
 * VPN and tunnel devices. A player's phone reaches the machine through a physical adapter, so the
 * e2e address comes from one of those. Mirrors the server's own virtual-adapter filter in intent
 * (the self-signed certificate lists only addresses a LAN peer could use). */
const VIRTUAL_NAME = /vethernet|virtualbox|vmware|hyper-v|docker|veth|^br-|tun|tap|nord|wireguard|utun|loopback|^lo$/i;

/** One non-loopback IPv4 address and the adapter that carries it. */
interface LanCandidate {
  /** The adapter name as `os.networkInterfaces()` reports it. */
  name: string;
  /** The adapter's IPv4 address. */
  address: string;
}

/** How a browser treats an IPv4 address when deciding whether to upgrade `http://` to `https://`:
 * engines exempt private (RFC 1918) addresses, so only `private` reproduces the LAN behaviour the
 * upgrade specs assert; a shared-address-space (CGNAT) or public address is upgraded like any
 * internet host, which changes the meaning of every "upgrade" cell. */
export type AddressClass = "private" | "cgnat" | "link-local" | "loopback" | "public";

/**
 * Classifies an IPv4 address by the ranges browsers key their local-address exemptions on:
 * RFC 1918 private space, RFC 6598 shared address space (`100.64.0.0/10`), RFC 3927 link-local
 * (`169.254.0.0/16`) and loopback; everything else is public.
 * @param address A dotted-quad IPv4 address.
 * @returns The address class.
 * @example
 * ```ts
 * classifyAddress("192.0.2.1"); // "public" (documentation range, not private)
 * classifyAddress("10.1.2.3"); // "private"
 * classifyAddress("100.64.0.9"); // "cgnat"
 * ```
 */
export function classifyAddress(address: string): AddressClass {
  const parts = address.split(".").map(Number);
  if (parts.length !== 4 || parts.some((p) => !Number.isInteger(p) || p < 0 || p > 255)) return "public";
  const [a, b] = parts;
  if (a === 127) return "loopback";
  if (a === 10) return "private";
  if (a === 172 && b >= 16 && b <= 31) return "private";
  if (a === 192 && b === 168) return "private";
  if (a === 100 && b >= 64 && b <= 127) return "cgnat";
  if (a === 169 && b === 254) return "link-local";
  return "public";
}

/** The address is private: run. */
export interface AddressOk {
  /** Discriminant. */
  kind: "ok";
}

/** Not private and the run is local: skip with this message. */
export interface AddressSkip {
  /** Discriminant. */
  kind: "skip";
  /** Why the test does not apply to this address. */
  message: string;
}

/** Not private and the run is on CI: fail with this message, so the gate never passes by skipping. */
export interface AddressFail {
  /** Discriminant. */
  kind: "fail";
  /** Why the address is unusable on CI. */
  message: string;
}

/** What a test that needs a private LAN address does with the address it got. */
export type AddressVerdict = AddressOk | AddressSkip | AddressFail;

/**
 * Decides whether a test whose meaning depends on a private (RFC 1918) address may run. Engines
 * upgrade CGNAT and public addresses like internet hosts, so the upgrade rows would mean something
 * else there; locally the test skips, and on CI it fails so a runner with such an address is
 * noticed instead of silently passing the suite.
 * @param cls The class of the address under test.
 * @param ci Whether the run is on CI.
 * @returns The verdict.
 * @example
 * ```ts
 * judgeAddress("cgnat", true).kind; // "fail"
 * judgeAddress("private", true).kind; // "ok"
 * ```
 */
export function judgeAddress(cls: AddressClass, ci: boolean): AddressVerdict {
  if (cls === "private") return { kind: "ok" };
  const message = `LAN address is ${cls}: engines upgrade it like an internet host, so the upgrade rows would mean something else`;
  return ci ? { kind: "fail", message } : { kind: "skip", message };
}

/**
 * Guards a test whose meaning depends on the address under test being private: records the
 * address class on the test, then skips (locally) or throws (on CI) per {@link judgeAddress}. A
 * host name, unlike an IP literal, is not classified and passes.
 * @param baseURL The project's base URL.
 * @param testInfo The running test, which receives an `address-class` annotation.
 * @param skip Skips the running test when its first argument is true, with the second as the
 * reason; called with `false` when the test may run.
 * @param ci Whether the run is on CI.
 * @throws On CI, when the address is not private.
 * @example
 * ```ts
 * declare const testInfo: import("@playwright/test").TestInfo;
 * guardPrivateAddress("http://10.0.0.5:32001", testInfo, () => {}, false);
 * ```
 */
export function guardPrivateAddress(
  baseURL: string,
  testInfo: TestInfo,
  skip: (condition: boolean, reason: string) => void,
  ci: boolean,
): void {
  const host = new URL(baseURL).hostname;
  if (!/^\d+\.\d+\.\d+\.\d+$/.test(host)) return;
  const cls = classifyAddress(host);
  testInfo.annotations.push({ type: "address-class", description: cls });
  const verdict = judgeAddress(cls, ci);
  if (verdict.kind === "fail") throw new Error(verdict.message);
  skip(verdict.kind === "skip", verdict.kind === "skip" ? verdict.message : "");
}

/**
 * Picks the machine's non-loopback IPv4 address the HTTPS e2e specs browse to. Loopback is
 * deliberately avoided: browsers treat it as potentially trustworthy, so it cannot reproduce the
 * plain-HTTP failure modes (no secure context, cookie and mixed-content behaviour) that a LAN
 * address exhibits. Link-local (`169.254.0.0/16`) addresses are skipped: an adapter that failed to
 * obtain a lease carries one, and no peer can reach it.
 * @param interfaces The result of `os.networkInterfaces()`.
 * @param override An explicit address (`SHADOWCAT_E2E_LAN_IP`); used verbatim when non-empty.
 * @returns The chosen address: the override, else the first non-internal, non-link-local IPv4
 * address on a non-virtual adapter, else the first on any adapter.
 * @throws When the machine has no usable IPv4 address and no override is given.
 * @example
 * ```ts
 * import { pickLanAddress } from "./lan-address";
 *
 * const addr = pickLanAddress(
 *   { Ethernet: [{ address: "192.168.1.20", family: "IPv4", internal: false } as never] },
 *   undefined,
 * );
 * // addr === "192.168.1.20"
 * ```
 */
export function pickLanAddress(
  interfaces: Record<string, NetworkInterfaceInfo[] | undefined>,
  override: string | undefined,
): string {
  if (override) return override;
  const candidates: LanCandidate[] = [];
  for (const [name, infos] of Object.entries(interfaces)) {
    for (const info of infos ?? []) {
      if (info.family === "IPv4" && !info.internal && classifyAddress(info.address) !== "link-local") {
        candidates.push({ name, address: info.address });
      }
    }
  }
  const physical = candidates.find((c) => !VIRTUAL_NAME.test(c.name));
  const chosen = physical ?? candidates[0];
  if (!chosen) {
    throw new Error(
      "no usable (non-loopback, non-link-local) IPv4 address found for the HTTPS e2e specs; set SHADOWCAT_E2E_LAN_IP",
    );
  }
  return chosen.address;
}
