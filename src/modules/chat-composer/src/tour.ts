import type { TourMeta } from "@shadowcat/core";

/** The dice tour: how to roll from the composer. Its targets are the `data-tour` anchors
 * `Composer` sets on its message box, roll-mode picker and document-link button; a step whose
 * anchor is not mounted is skipped. */
export const diceTour: TourMeta = {
  id: "chat-composer.dice",
  labelKey: "chat-composer.tour.name",
  version: 1,
  audience: "any",
  scope: "global",
  trigger: "first-use",
  steps: [
    { target: "chat-composer.input", titleKey: "chat-composer.tour.input.title", bodyKey: "chat-composer.tour.input.body", placement: "top" },
    { target: "chat-composer.roll-mode", titleKey: "chat-composer.tour.rollMode.title", bodyKey: "chat-composer.tour.rollMode.body", placement: "top" },
    { target: "chat-composer.doc-link", titleKey: "chat-composer.tour.docLink.title", bodyKey: "chat-composer.tour.docLink.body", placement: "top" },
  ],
};
