import type { TourMeta } from "@shadowcat/core";

/** The stage's guided tour. Its targets are the `data-tour` anchors `Stage` sets on its host and
 * on the GM-only vision picker; the picker step is skipped for a player. */
export const stageTour: TourMeta = {
  id: "stage.intro",
  labelKey: "stage.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "first-use",
  steps: [
    { target: "stage.canvas", titleKey: "stage.tour.canvas.title", bodyKey: "stage.tour.canvas.body", placement: "center" },
    { target: "stage.gm-view", titleKey: "stage.tour.gmView.title", bodyKey: "stage.tour.gmView.body", placement: "bottom" },
  ],
};
