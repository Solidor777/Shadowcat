import type { TourMeta } from "@shadowcat/core";

/** The settings panel's guided tour, run from the replay list: the panel is closed by default, so
 * it is never offered automatically. Its targets are the `data-tour` anchors `Settings` sets on
 * its own root and language picker. */
export const settingsTour: TourMeta = {
  id: "settings.intro",
  labelKey: "settings.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "manual",
  steps: [
    { target: "settings.panel", titleKey: "settings.tour.panel.title", bodyKey: "settings.tour.panel.body", placement: "left" },
    { target: "settings.language", titleKey: "settings.tour.language.title", bodyKey: "settings.tour.language.body", placement: "bottom" },
  ],
};
