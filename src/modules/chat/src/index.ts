import type { Module } from "@shadowcat/core";
import ChatPanel from "./ChatPanel.svelte";

/** Chat panel host: contributes the default (order 0) sidebar tab and declares the
 * two singleton surfaces a composer/card module fills (chat.composer, chat.message).
 * Renders gracefully with neither filled — an empty registry read yields nothing in
 * those slots — so this module lands independently of the modules that fill them. */
export const chat: Module = {
  manifest: {
    id: "chat",
    version: "0.1.0",
    dependencies: {},
    requires: ["shadowcat.surface:sidebar"],
    provides: [
      { contract: "shadowcat.surface:chat.composer", cardinality: "singleton" },
      { contract: "shadowcat.surface:chat.message", cardinality: "singleton" },
    ],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "chat:sidebar", contract: "shadowcat.surface:sidebar", order: 0, component: ChatPanel, tab: { icon: "💬", labelKey: "chat.tab" } });
  },
};
