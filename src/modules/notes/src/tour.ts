import type { TourMeta } from "@shadowcat/core";

/** The notes panel's guided tour, run from the replay list: the panel is closed by default, so
 * it is never offered automatically. Its targets are the `data-tour` anchors `NotesPanel` sets on
 * its own root and search box. */
export const notesTour: TourMeta = {
  id: "notes.intro",
  labelKey: "notes.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "manual",
  steps: [
    { target: "notes.panel", titleKey: "notes.tour.panel.title", bodyKey: "notes.tour.panel.body", placement: "left" },
    { target: "notes.search", titleKey: "notes.tour.search.title", bodyKey: "notes.tour.search.body", placement: "bottom" },
  ],
};
