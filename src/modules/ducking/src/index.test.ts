import { describe, it, expect, afterEach } from "vitest";
import { ContributionRegistry, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
import { ducking } from "./index";
import { makeBindings } from "./__fixtures__/bindings";

describe("ducking module", () => {
  afterEach(() => {
    ducking.unregister?.();
    localStorage.clear();
  });

  it("requires SETTINGS_SECTION_CONTRACT and the always-mounted overlay surface, and provides nothing", () => {
    expect(ducking.manifest.requires).toEqual([
      "shadowcat.settings-section",
      "shadowcat.surface:overlay",
    ]);
    expect(ducking.manifest.provides).toEqual([]);
  });

  it("contributes its settings section with the expected metadata", () => {
    const contributions = new ContributionRegistry();
    ducking.register({ contributions, keybindings: makeBindings(new EventTarget()).bindings } as never);
    const list = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe("ducking:settings");
    expect(list[0].settingsSection).toEqual({ labelKey: "ducking.sectionTitle" });
  });

  it("contributes its runtime wiring into the overlay surface", () => {
    const contributions = new ContributionRegistry();
    ducking.register({ contributions, keybindings: makeBindings(new EventTarget()).bindings } as never);
    const list = contributions.contributionsFor("shadowcat.surface:overlay");
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe("ducking:runtime");
  });

  it("registers its push-to-duck binding through ctx.keybindings", () => {
    const { controller, bindings } = makeBindings(new EventTarget());
    ducking.register({ contributions: new ContributionRegistry(), keybindings: bindings } as never);
    expect(controller.bindingsFor("ducking").map((b) => b.bindingId)).toEqual(["push-to-duck"]);
  });
});
