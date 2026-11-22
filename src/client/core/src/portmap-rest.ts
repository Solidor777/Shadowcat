import type { PortMapStatus } from "@shadowcat/types";
import { restError } from "./user-rest";

// Admin port-forwarding REST: both routes are gated on ServerRole::Admin (`http::port_forward`).
// The status carries the host's external address, so it is only ever fetched from an admin-only
// surface. Nothing here is a permission check; a non-admin caller gets a 403 the UI surfaces as an
// error.

/** Reads the port-forwarding status. Admin-only.
 * @returns The current status: toggle, lock, task state and any granted mapping.
 * @example
 * ```ts
 * import { getPortMapStatus } from "@shadowcat/core";
 *
 * const status = await getPortMapStatus();
 * const mapped = status.state === "mapped";
 * ```
 */
export async function getPortMapStatus(): Promise<PortMapStatus> {
  const res = await fetch("/api/admin/portmap", { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(await restError(res, "port forwarding status failed"));
  return (await res.json()) as PortMapStatus;
}

/** Stores the port-forwarding toggle and applies it at once. Admin-only. The server answers 409
 * when the `port_forward` configuration key fixes the toggle.
 * @param enabled - Whether the server should map its port on the router.
 * @returns The status the server reports once the toggle is applied.
 * @example
 * ```ts
 * import { setPortForward } from "@shadowcat/core";
 *
 * const status = await setPortForward(true);
 * ```
 */
export async function setPortForward(enabled: boolean): Promise<PortMapStatus> {
  const res = await fetch("/api/admin/portmap", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ enabled }),
  });
  if (!res.ok) throw new Error(await restError(res, "set port forwarding failed"));
  return (await res.json()) as PortMapStatus;
}
