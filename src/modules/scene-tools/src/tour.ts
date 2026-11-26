import type { TourMeta } from "@shadowcat/core";

/** The tool rail's guided tour. Its targets are the `data-tour` anchors `ToolRail` sets on its
 * root and on each built-in tool button; the place step is skipped for a player, whose rail does
 * not mount that button. */
export const sceneToolsTour: TourMeta = {
  id: "scene-tools.intro",
  labelKey: "scene-tools.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "first-use",
  steps: [
    { target: "scene-tools.rail", titleKey: "scene-tools.tour.rail.title", bodyKey: "scene-tools.tour.rail.body", placement: "right" },
    { target: "scene-tools.select", titleKey: "scene-tools.tour.select.title", bodyKey: "scene-tools.tour.select.body", placement: "right" },
    { target: "scene-tools.place", titleKey: "scene-tools.tour.place.title", bodyKey: "scene-tools.tour.place.body", placement: "right" },
    { target: "scene-tools.measure", titleKey: "scene-tools.tour.measure.title", bodyKey: "scene-tools.tour.measure.body", placement: "right" },
    { target: "scene-tools.ping", titleKey: "scene-tools.tour.ping.title", bodyKey: "scene-tools.tour.ping.body", placement: "right" },
  ],
};
