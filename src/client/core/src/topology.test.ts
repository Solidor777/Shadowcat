import { describe, it, expect, vi } from "vitest";
import { reconcileTopology } from "./topology";
import type { Logger } from "./logger";

const decl = (
  module_id: string,
  opts?: { version?: string; provides?: { contract: string; cardinality: "singleton" | "multi" }[]; requires?: string[] },
) => ({
  module_id,
  version: opts?.version ?? "1",
  provides: opts?.provides ?? [],
  requires: opts?.requires ?? [],
});
// Logger is { debug, warn, error } — no `info`.
const logger = (): Logger => ({ debug: vi.fn(), warn: vi.fn(), error: vi.fn() });

describe("reconcileTopology", () => {
  it("does not warn when local and remote module sets match", () => {
    const l = logger();
    reconcileTopology([decl("a"), decl("b")], [decl("a"), decl("b")], l);
    expect(l.warn).not.toHaveBeenCalled();
  });

  it("does not warn when matching module_id sets also match version, provides, and requires", () => {
    const l = logger();
    const shared = decl("a", {
      version: "2.1.0",
      provides: [{ contract: "shadowcat.panel", cardinality: "singleton" }],
      requires: ["shadowcat.sheet:actor"],
    });
    reconcileTopology([shared], [shared], l);
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

  it("warns exactly once for a module present on both sides with a different version", () => {
    const l = logger();
    reconcileTopology(
      [decl("a", { version: "1.0.0" })],
      [decl("a", { version: "1.1.0" })],
      l,
    );
    expect(l.warn).toHaveBeenCalledTimes(1);
    expect(l.warn).toHaveBeenCalledWith(expect.stringContaining("1.0.0"));
    expect(l.warn).toHaveBeenCalledWith(expect.stringContaining("1.1.0"));
  });

  it("warns when local provides has a contract remote does not declare", () => {
    const l = logger();
    reconcileTopology(
      [decl("a", { provides: [{ contract: "shadowcat.panel", cardinality: "singleton" }] })],
      [decl("a", { provides: [] })],
      l,
    );
    expect(l.warn).toHaveBeenCalledTimes(1);
    expect(l.warn).toHaveBeenCalledWith(expect.stringContaining("shadowcat.panel"));
  });

  it("warns when a shared provides contract id has a different cardinality on each side", () => {
    const l = logger();
    reconcileTopology(
      [decl("a", { provides: [{ contract: "shadowcat.panel", cardinality: "singleton" }] })],
      [decl("a", { provides: [{ contract: "shadowcat.panel", cardinality: "multi" }] })],
      l,
    );
    expect(l.warn).toHaveBeenCalledTimes(1);
    expect(l.warn).toHaveBeenCalledWith(expect.stringContaining("cardinality"));
  });

  it("warns when requires differs between local and remote", () => {
    const l = logger();
    reconcileTopology(
      [decl("a", { requires: ["shadowcat.sheet:actor"] })],
      [decl("a", { requires: [] })],
      l,
    );
    expect(l.warn).toHaveBeenCalledTimes(1);
    expect(l.warn).toHaveBeenCalledWith(expect.stringContaining("shadowcat.sheet:actor"));
  });
});
