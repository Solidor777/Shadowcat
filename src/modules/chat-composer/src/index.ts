import type { Module } from "@shadowcat/core";
import Composer from "./Composer.svelte";

/** Default composer for the singleton chat.composer surface: an auto-growing
 * textarea that sends via ctx.chat.send. Fills chat's declared singleton slot;
 * a game-system module may provide its own composer instead. */
export const chatComposer: Module = {
  manifest: {
    id: "chat-composer",
    version: "0.1.0",
    dependencies: {},
    requires: ["shadowcat.surface:chat.composer"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "chat-composer:main", contract: "shadowcat.surface:chat.composer", component: Composer });
  },
};
