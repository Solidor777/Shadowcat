import type { Module } from "@shadowcat/core";
import MessageCard from "./MessageCard.svelte";

/** The default message-card renderer: fills the singleton `chat.message` surface `chat`
 * declares. Fail-closed body parse + the sole `{@html}` boundary in the client for chat
 * content live in `MessageCard.svelte`. Replaceable — a game-system module can supply its
 * own renderer by contributing to the same contract. */
export const chatCard: Module = {
  manifest: {
    id: "chat-card",
    version: "0.1.0",
    dependencies: {},
    requires: ["shadowcat.surface:chat.message"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "chat-card:main", contract: "shadowcat.surface:chat.message", component: MessageCard });
  },
};
