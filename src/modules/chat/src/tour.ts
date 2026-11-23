import type { TourMeta } from "@shadowcat/core";

/** The chat panel's guided tour. Its targets are the `data-tour` anchors `ChatPanel` sets on its
 * own root, channel strip and message list; a step whose anchor is not mounted is skipped. */
export const chatTour: TourMeta = {
  id: "chat.intro",
  labelKey: "chat.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "first-use",
  steps: [
    { target: "chat.panel", titleKey: "chat.tour.panel.title", bodyKey: "chat.tour.panel.body", placement: "left" },
    { target: "chat.channels", titleKey: "chat.tour.channels.title", bodyKey: "chat.tour.channels.body", placement: "bottom" },
    { target: "chat.messages", titleKey: "chat.tour.messages.title", bodyKey: "chat.tour.messages.body", placement: "left" },
  ],
};
