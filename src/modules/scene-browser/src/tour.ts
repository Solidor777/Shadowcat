import type { TourMeta } from "@shadowcat/core";

/** The scene browser's guided tour for GMs, run from the replay list: the panel is closed by
 * default, so it is never offered automatically. Its targets are the `data-tour` anchors
 * `SceneBrowserPanel` sets on its root, list, create button and first row's action group; the
 * actions step is skipped while the world has no scene. */
export const sceneBrowserTour: TourMeta = {
  id: "scene-browser.intro",
  labelKey: "sceneBrowser.tour.name",
  version: 1,
  audience: "gm",
  scope: "global",
  trigger: "manual",
  steps: [
    { target: "scene-browser.panel", titleKey: "sceneBrowser.tour.panel.title", bodyKey: "sceneBrowser.tour.panel.body", placement: "left" },
    { target: "scene-browser.create", titleKey: "sceneBrowser.tour.create.title", bodyKey: "sceneBrowser.tour.create.body", placement: "top" },
    { target: "scene-browser.list", titleKey: "sceneBrowser.tour.list.title", bodyKey: "sceneBrowser.tour.list.body", placement: "left" },
    { target: "scene-browser.actions", titleKey: "sceneBrowser.tour.actions.title", bodyKey: "sceneBrowser.tour.actions.body", placement: "bottom" },
  ],
};
