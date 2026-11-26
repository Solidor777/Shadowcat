import { describe, expect, it, vi } from "vitest";
import { ContributionRegistry, SETTINGS_SECTION_CONTRACT, TOUR_CONTRACT } from "@shadowcat/core";
import { onboarding } from "./index";
import { memorySettings } from "./__fixtures__/harness";

describe("onboarding module", () => {
  it("requires the settings-section and overlay surfaces and provides the tour contract", () => {
    expect(onboarding.manifest.id).toBe("onboarding");
    expect(onboarding.manifest.requires).toEqual([SETTINGS_SECTION_CONTRACT, "shadowcat.surface:overlay"]);
    expect(onboarding.manifest.provides).toEqual([{ contract: TOUR_CONTRACT, cardinality: "multi" }]);
  });

  it("mounts the engine in the overlay surface and the replay list as a settings section", () => {
    const contributions = new ContributionRegistry();
    onboarding.register({ contributions, settings: memorySettings(), logger: { warn: vi.fn() } } as never);
    const overlay = contributions.contributionsFor("shadowcat.surface:overlay");
    expect(overlay.map((c) => c.id)).toEqual(["onboarding:host"]);
    const sections = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
    expect(sections.map((c) => c.id)).toEqual(["onboarding:replay"]);
    expect(sections[0].settingsSection).toEqual({ labelKey: "onboarding.replay.title" });
  });

  it("hands the host and the replay list the same controller", () => {
    const contributions = new ContributionRegistry();
    onboarding.register({ contributions, settings: memorySettings(), logger: { warn: vi.fn() } } as never);
    const host = contributions.contributionsFor("shadowcat.surface:overlay")[0];
    const replay = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT)[0];
    expect(host.props?.controller).toBeDefined();
    expect(host.props?.controller).toBe(replay.props?.controller);
  });

  it("logs a rejected progress write through the module logger", async () => {
    const warn = vi.fn();
    const contributions = new ContributionRegistry();
    const failure = new Error("offline");
    onboarding.register({
      contributions,
      settings: { getUser: () => ({}), setUser: () => Promise.reject(failure) },
      logger: { warn },
    } as never);
    const controller = contributions.contributionsFor("shadowcat.surface:overlay")[0].props?.controller as {
      disableForever(): void;
    };
    controller.disableForever();
    await Promise.resolve();
    await Promise.resolve();
    expect(warn).toHaveBeenCalledWith("onboarding: could not save tour progress", failure);
  });
});
