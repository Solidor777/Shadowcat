import type { AppContext } from "../appContext";
import { __APP_CONTEXT_KEY__ } from "../appContext";
import { DocumentStore, AssetResolver, ContributionRegistry } from "@shadowcat/core";
import { SceneInteractionBridge } from "../sceneInteraction";
import { ActorSelection } from "../actorSelection.svelte";
import { TokenSelection } from "../tokenSelection.svelte";

/** Build a Map for @testing-library/svelte's `context` option holding a minimal
 * AppContext (overridable per field), seeded under the real private key. */
export function setAppContextForTest(over: Partial<AppContext> = {}): Map<unknown, unknown> {
  const ctx: AppContext = {
    contributions: over.contributions ?? new ContributionRegistry(),
    store: over.store ?? new DocumentStore(),
    documents: over.documents ?? over.store ?? new DocumentStore(),
    assets: over.assets ?? new AssetResolver(),
    world: over.world ?? "w1",
    role: over.role ?? "gm",
    selfId: over.selfId ?? "u-self",
    canEdit: over.canEdit ?? (() => true),
    members: over.members ?? new Map(),
    t: over.t ?? ((k: string) => k),
    onAssetChanged: over.onAssetChanged ?? (() => () => {}),
    subscribeScene: over.subscribeScene ?? (() => ({ unsubscribe() {} })),
    dispatchIntent: over.dispatchIntent ?? (() => {}),
    scene: over.scene ?? new SceneInteractionBridge(),
    actorSelection: over.actorSelection ?? new ActorSelection(),
    tokenSelection: over.tokenSelection ?? new TokenSelection(),
    sendPing: over.sendPing ?? (() => {}),
    onPing: over.onPing ?? (() => () => {}),
    leaveWorld: over.leaveWorld ?? (() => {}),
    logout: over.logout ?? (async () => {}),
  };
  return new Map([[__APP_CONTEXT_KEY__, ctx]]);
}
