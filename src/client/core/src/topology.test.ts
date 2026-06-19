import { describe, it, expect, vi } from "vitest";
import { reconcileTopology } from "./topology";
import type { Logger } from "./logger";

const decl = (module_id: string) => ({ module_id, version: "1", provides: [], requires: [] });
// Logger is { debug, warn, error } — no `info`.
const logger = (): Logger => ({ debug: vi.fn(), warn: vi.fn(), error: vi.fn() });

describe("reconcileTopology", () => {
  it("does not warn when local and remote module sets match", () => {
    const l = logger();
    reconcileTopology([decl("a"), decl("b")], [decl("a"), decl("b")], l);
    expect(l.warn).not.toHaveBeenCalled();
  });

  it("warns for a module loaded locally but absent from the world topology", () => {
    const l = logger();
    reconcileTopology([decl("a"), decl("x")], [decl("a")], l);
    expect(l.warn).toHaveBeenCalledTimes(1);
  });

  it("warns for a module in the world topology but not loaded locally", () => {
    const l = logger();
    reconcileTopology([decl("a")], [decl("a"), decl("y")], l);
    expect(l.warn).toHaveBeenCalledTimes(1);
  });
});
