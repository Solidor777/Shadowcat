import type { HealthStatus } from "@shadowcat/types";

/** Returns true when the server reports itself healthy with a live database. */
export function isHealthy(status: HealthStatus): boolean {
  return status.status === "ok" && status.db_connected;
}
