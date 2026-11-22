import { SETTINGS_SECTION_CONTRACT, STAGE_OVERLAY_CONTRACT, type Module } from "@shadowcat/core";
import { registerSetting } from "@shadowcat/module-settings";
import BubbleOverlay from "./BubbleOverlay.svelte";
import { ChatBubblesPreference } from "./preference.svelte";

export {
  BUBBLE_MS,
  MAX_BUBBLES,
  seedSeen,
  scanNewMessages,
  tokenInViewedScope,
  resolveSpeakerToken,
  addBubble,
} from "./bubbles";
export type { Bubble, BubbleMessage } from "./bubbles";
export { ChatBubblesPreference, CHAT_BUBBLES_STORAGE_KEY } from "./preference.svelte";
export { default as BubbleOverlay } from "./BubbleOverlay.svelte";

/** Chat bubbles: a speech bubble over the token that just spoke, drawn in the stage overlay
 * layer, plus a per-device settings toggle. Strictly additive to the chat panel, which still
 * shows every message; removing the module removes only the bubbles. */
export const chatBubbles: Module = {
  manifest: {
    id: "chat-bubbles",
    version: "0.1.0",
    dependencies: {},
    requires: [SETTINGS_SECTION_CONTRACT],
    provides: [{ contract: STAGE_OVERLAY_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    const preference = new ChatBubblesPreference();
    ctx.contributions.contribute({
      id: "chat-bubbles:overlay",
      contract: STAGE_OVERLAY_CONTRACT,
      component: BubbleOverlay,
      props: { preference },
    });
    registerSetting(ctx, {
      id: "chat-bubbles:setting",
      labelKey: "chatBubbles.setting",
      field: { kind: "toggle", get: () => preference.enabled, set: (value) => preference.set(value) },
    });
  },
};
