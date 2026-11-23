import type { TourMeta } from "@shadowcat/core";

/** The actors panel's guided tour, run from the replay list: the panel is closed by default, so
 * it is never offered automatically. Its targets are the `data-tour` anchors `ActorsPanel` sets on
 * its root, search box and list, and `TokenQuickEdit` on its section; the quick-edit step is
 * skipped while no single token is selected. */
export const actorsTour: TourMeta = {
  id: "actors.intro",
  labelKey: "actors.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "manual",
  steps: [
    { target: "actors.panel", titleKey: "actors.tour.panel.title", bodyKey: "actors.tour.panel.body", placement: "left" },
    { target: "actors.search", titleKey: "actors.tour.search.title", bodyKey: "actors.tour.search.body", placement: "bottom" },
    { target: "actors.list", titleKey: "actors.tour.list.title", bodyKey: "actors.tour.list.body", placement: "left" },
    { target: "actors.quick-edit", titleKey: "actors.tour.quickEdit.title", bodyKey: "actors.tour.quickEdit.body", placement: "left" },
  ],
};
