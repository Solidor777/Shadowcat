import { describe, expect, it } from "vitest";
import { TourController } from "./controller.svelte";
import { TourProgress } from "./progress";
import { makeController, memorySettings, threeStepTour, tour } from "./__fixtures__/harness";

describe("TourController.evaluate", () => {
  it("offers the first eligible tour", async () => {
    const { controller } = makeController({ tours: [tour()], mounted: ["example.a"] });
    await controller.evaluate();
    expect(controller.offer?.id).toBe("example.intro");
  });

  it("offers nothing before the host attaches an environment", async () => {
    const controller = new TourController({
      progress: new TourProgress(memorySettings(), () => {}),
      serverEnabled: async () => true,
    });
    await controller.evaluate();
    expect(controller.offer).toBeNull();
  });

  it("offers nothing when the server disallows tours", async () => {
    const { controller } = makeController({ tours: [tour()], mounted: ["example.a"], serverEnabled: false });
    await controller.evaluate();
    expect(controller.offer).toBeNull();
  });

  it("offers nothing after Don't ask again was chosen in an earlier session", async () => {
    const settings = memorySettings({ disabled: true });
    const { controller } = makeController({ tours: [tour()], mounted: ["example.a"], settings });
    await controller.evaluate();
    expect(controller.offer).toBeNull();
  });

  it("makes at most one offer per session", async () => {
    const tours = [tour({ id: "example.one" }), tour({ id: "example.two" })];
    const { controller } = makeController({ tours, mounted: ["example.a"] });
    await controller.evaluate();
    controller.dismissOffer();
    await controller.evaluate();
    expect(controller.offer).toBeNull();
  });

  it("does not offer a tour that is already finished at its current version", async () => {
    const settings = memorySettings({ "g:example.intro": 1 });
    const { controller } = makeController({ tours: [tour()], mounted: ["example.a"], settings });
    await controller.evaluate();
    expect(controller.offer).toBeNull();
  });
});

describe("TourController answers to the offer", () => {
  it("Not now closes the offer and records the tour as done, so a later session does not offer it again", async () => {
    const first = makeController({ tours: [tour()], mounted: ["example.a"] });
    await first.controller.evaluate();
    first.controller.dismissOffer();
    expect(first.controller.offer).toBeNull();
    expect(first.settings.writes).toEqual([{ "g:example.intro": 1 }]);

    const nextSession = makeController({
      tours: [tour()],
      mounted: ["example.a"],
      settings: memorySettings({ "g:example.intro": 1 }),
    });
    await nextSession.controller.evaluate();
    expect(nextSession.controller.offer).toBeNull();
  });

  it("Don't ask again records the switch and closes the offer", async () => {
    const { controller, settings } = makeController({ tours: [tour()], mounted: ["example.a"] });
    await controller.evaluate();
    controller.disableForever();
    expect(controller.offer).toBeNull();
    expect(settings.writes).toEqual([{ disabled: true }]);
  });

  it("Start closes the offer and shows the first step", async () => {
    const { controller } = makeController({ tours: [tour()], mounted: ["example.a"] });
    await controller.evaluate();
    controller.startOffer();
    expect(controller.offer).toBeNull();
    expect(controller.active?.stepIndex).toBe(0);
  });
});

describe("TourController stepping", () => {
  it("skips steps whose target is not mounted, forward and back", () => {
    const { controller } = makeController({ tours: [], mounted: ["example.a", "example.c"] });
    expect(controller.start(threeStepTour())).toBe(true);
    expect(controller.active?.stepIndex).toBe(0);
    controller.next();
    expect(controller.active?.stepIndex).toBe(2);
    expect(controller.active?.shown).toEqual([0, 2]);
    controller.back();
    expect(controller.active?.stepIndex).toBe(0);
  });

  it("starts at the first step that can be shown", () => {
    const { controller } = makeController({ tours: [], mounted: ["example.b"] });
    controller.start(threeStepTour());
    expect(controller.active?.stepIndex).toBe(1);
  });

  it("refuses to start a tour with no step to show", () => {
    const { controller } = makeController({ tours: [], mounted: [] });
    expect(controller.start(threeStepTour())).toBe(false);
    expect(controller.active).toBeNull();
  });

  it("stays on the first step when Back is pressed there", () => {
    const { controller } = makeController({ tours: [], mounted: ["example.a", "example.b", "example.c"] });
    controller.start(threeStepTour());
    controller.back();
    expect(controller.active?.stepIndex).toBe(0);
  });

  it("ends and records the tour when Next is pressed on the last step", () => {
    const { controller, settings } = makeController({ tours: [], mounted: ["example.a"] });
    controller.start(tour({ version: 4 }));
    controller.next();
    expect(controller.active).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 4 }]);
  });

  it("records a skipped tour the same as a finished one", () => {
    const { controller, settings } = makeController({ tours: [], mounted: ["example.a", "example.b"] });
    controller.start(threeStepTour());
    controller.end();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
  });

  it("records a world-scoped tour under the viewer's world", () => {
    const { controller, settings } = makeController({ tours: [], mounted: ["example.a"] });
    controller.start(tour({ scope: "world" }));
    controller.end();
    expect(settings.writes).toEqual([{ "w:w1:example.intro": 1 }]);
  });
});

describe("TourController across sessions", () => {
  it("a second session over the same stored state does not offer a finished tour", async () => {
    const settings = memorySettings();
    const first = makeController({ tours: [tour()], mounted: ["example.a"], settings });
    await first.controller.evaluate();
    first.controller.startOffer();
    first.controller.end();

    const second = makeController({ tours: [tour()], mounted: ["example.a"], settings });
    await second.controller.evaluate();
    expect(second.controller.offer).toBeNull();
  });

  it("a bumped version is offered again to a user who finished the old one", async () => {
    const settings = memorySettings();
    const first = makeController({ tours: [tour({ version: 1 })], mounted: ["example.a"], settings });
    await first.controller.evaluate();
    first.controller.startOffer();
    first.controller.end();

    const second = makeController({ tours: [tour({ version: 2 })], mounted: ["example.a"], settings });
    await second.controller.evaluate();
    expect(second.controller.offer?.version).toBe(2);
  });
});
