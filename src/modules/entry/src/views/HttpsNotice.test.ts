import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { TlsStatus } from "@shadowcat/types";
import HttpsNotice from "./HttpsNotice.svelte";

const TLS: TlsStatus = {
  enabled: true,
  source: "self-signed",
  https_origins: [],
  alpn: ["h2", "http/1.1"],
};
const LAN = {
  protocol: "http:",
  hostname: "192.168.1.20",
  host: "192.168.1.20:30000",
  pathname: "/",
  search: "",
  hash: "",
  origin: "http://192.168.1.20:30000",
};

function memoryStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
  };
}

test("offers the https address and the one-time warning note for a self-signed certificate", () => {
  render(HttpsNotice, { tls: TLS, loc: LAN, storage: memoryStorage() });
  expect(screen.getByTestId("https-notice-link").getAttribute("href")).toBe(
    "https://192.168.1.20:30000/",
  );
  expect(screen.getByText(/your browser will warn once/)).toBeTruthy();
});

test("omits the warning note for a non-self-signed source", () => {
  render(HttpsNotice, { tls: { ...TLS, source: "operator" }, loc: LAN, storage: memoryStorage() });
  expect(screen.queryByText(/your browser will warn once/)).toBeNull();
});

test("renders nothing on loopback", () => {
  render(HttpsNotice, { tls: TLS, loc: { ...LAN, hostname: "localhost" }, storage: memoryStorage() });
  expect(screen.queryByTestId("https-notice")).toBeNull();
});

test("dismissal hides the notice and persists per origin", async () => {
  const storage = memoryStorage();
  const first = render(HttpsNotice, { tls: TLS, loc: LAN, storage });
  await fireEvent.click(screen.getByTestId("https-notice-dismiss"));
  expect(screen.queryByTestId("https-notice")).toBeNull();
  first.unmount();
  render(HttpsNotice, { tls: TLS, loc: LAN, storage });
  expect(screen.queryByTestId("https-notice")).toBeNull();
});

test("a storage that throws does not break rendering or dismissal", async () => {
  const boom = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  };
  render(HttpsNotice, { tls: TLS, loc: LAN, storage: boom });
  expect(screen.getByTestId("https-notice")).toBeTruthy();
  await fireEvent.click(screen.getByTestId("https-notice-dismiss"));
  expect(screen.queryByTestId("https-notice")).toBeNull();
});
