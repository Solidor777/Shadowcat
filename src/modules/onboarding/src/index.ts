import { SETTINGS_SECTION_CONTRACT, TOUR_CONTRACT, type Module } from "@shadowcat/core";
import { TourController } from "./controller.svelte";
import OnboardingHost from "./OnboardingHost.svelte";
import { TourProgress } from "./progress";
import ReplaySection from "./ReplaySection.svelte";
import { fetchOnboardingEnabled } from "./serverFlag";

export { TourController, type ActiveTour, type TourControllerDeps, type TourEnvironment } from "./controller.svelte";
export { matchesAudience, offerableTours, resolvableSteps, type AnchorResolver, type TourViewer } from "./eligibility";
export { appEnvironment, registeredTours, viewerOf } from "./environment";
export { placeCard, type Box } from "./placement";
export { TourProgress, doneKey, type ProgressSettings } from "./progress";
export { fetchOnboardingEnabled } from "./serverFlag";
export { default as OnboardingHost } from "./OnboardingHost.svelte";
export { default as ReplaySection } from "./ReplaySection.svelte";
export { default as TourOffer } from "./TourOffer.svelte";
export { default as TourOverlay } from "./TourOverlay.svelte";

/** First-run guided tours. Any module contributes a tour under `TOUR_CONTRACT`; this module is the
 * engine that offers eligible tours (a dismissible toast, never a modal), runs the one the user
 * starts as a spotlight plus step card, records finished or skipped tours per account, and lists
 * every tour in a "Replay tours" settings section. Mounts through the app-level overlay surface and
 * contributes its settings section through `SETTINGS_SECTION_CONTRACT`. Removing the module removes
 * every tour offer and leaves the contributed tours inert. */
export const onboarding: Module = {
  manifest: {
    id: "onboarding",
    version: "0.1.0",
    dependencies: {},
    requires: [SETTINGS_SECTION_CONTRACT, "shadowcat.surface:overlay"],
    provides: [{ contract: TOUR_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    const progress = new TourProgress(ctx.settings, (error) => ctx.logger.warn("onboarding: could not save tour progress", error));
    // The server flag is fixed for the life of the process, so one read serves every decision.
    let enabled: Promise<boolean> | null = null;
    const controller = new TourController({
      progress,
      serverEnabled: () => (enabled ??= fetchOnboardingEnabled()),
    });
    ctx.contributions.contribute({
      id: "onboarding:host",
      contract: "shadowcat.surface:overlay",
      component: OnboardingHost,
      props: { controller },
    });
    ctx.contributions.contribute({
      id: "onboarding:replay",
      contract: SETTINGS_SECTION_CONTRACT,
      component: ReplaySection,
      props: { controller },
      settingsSection: { labelKey: "onboarding.replay.title" },
    });
  },
};
