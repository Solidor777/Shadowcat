import { CHAT_CARD_EXTENSION_CONTRACT, type Module } from "@shadowcat/core";
import MessageCard from "./MessageCard.svelte";

/** The default message-card renderer: fills the singleton `chat.message` surface `chat`
 * declares. Fail-closed body parsing lives in `MessageCard`; the sole `{@html}` boundary
 * for chat content is `SegmentList` (ui-kit), which `MessageCard` delegates segment
 * rendering to. Replaceable — a game-system module can supply its
 * own renderer by contributing to the same contract. Also PROVIDES `shadowcat.chat-card-extension`
 * — the seam `MessageCard` renders after its own built-in content, once per matching contribution,
 * against the message's opaque `system` payload (any module may
 * `requires: [CHAT_CARD_EXTENSION_CONTRACT]` and contribute one). */
export const chatCard: Module = {
  manifest: {
    id: "chat-card",
    version: "0.1.0",
    dependencies: {},
    requires: ["shadowcat.surface:chat.message"],
    provides: [{ contract: CHAT_CARD_EXTENSION_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "chat-card:main", contract: "shadowcat.surface:chat.message", component: MessageCard });
  },
};
