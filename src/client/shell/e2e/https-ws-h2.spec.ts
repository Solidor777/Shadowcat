import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { loginAt } from "./https-support";

// The server advertises its ALPN list in `GET /api/config` (`TlsStatus.alpn`). Each browser must
// negotiate a protocol from that list for page resources, negotiate `h2` when it is offered, and
// still complete the WebSocket handshake over `wss:` (RFC 8441 extended CONNECT when the
// connection is h2, or a fresh http/1.1 upgrade otherwise). The published list is the server
// truth: no copy of it lives in this file.

/**
 * Reads the published `TlsStatus.alpn` list, the protocol resource timing reports for
 * `/api/config`, and whether a `wss:` connection reaches the server Welcome frame.
 * @param page A page signed in on the https origin.
 * @returns The list, the negotiated protocol, and the first frame type received (or a failure
 * marker).
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const { alpn, nextHop, welcome } = await probe(page);
 * ```
 */
async function probe(page: Page) {
  return page.evaluate(async () => {
    const config = await (await fetch("/api/config")).json();
    const entry = performance
      .getEntriesByType("resource")
      .filter((e) => e.name.endsWith("/api/config"))
      .pop() as PerformanceResourceTiming | undefined;
    const created = await (
      await fetch("/api/worlds", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name: `ws-h2-${Date.now()}` }),
      })
    ).json();
    const welcome = await new Promise<string>((resolve) => {
      const ws = new WebSocket(`wss://${location.host}/ws?world=${created.id}`);
      const timer = setTimeout(() => resolve("timeout"), 15_000);
      ws.onopen = () =>
        ws.send(JSON.stringify({ type: "hello", world: created.id, last_seq: null }));
      ws.onmessage = (m) => {
        clearTimeout(timer);
        ws.close();
        resolve(String(JSON.parse(m.data as string).type));
      };
      ws.onerror = () => {
        clearTimeout(timer);
        resolve("error");
      };
    });
    return { alpn: config.tls.alpn as string[], nextHop: entry?.nextHopProtocol ?? "", welcome };
  });
}

test("page resources negotiate a protocol the server published and the wss welcome arrives", async ({
  page,
  baseURL,
}, testInfo) => {
  await loginAt(page, baseURL!);
  const result = await probe(page);
  testInfo.annotations.push({ type: "next-hop-protocol", description: result.nextHop });
  expect(result.alpn.length).toBeGreaterThan(0);
  expect(result.alpn).toContain(result.nextHop);
  if (result.alpn.includes("h2")) expect(result.nextHop).toBe("h2");
  expect(result.welcome).toBe("welcome");
});
