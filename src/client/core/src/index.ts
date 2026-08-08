import type { HealthStatus } from "@shadowcat/types";

/** Returns true when the server reports itself healthy with a live database.
 * @param status The parsed `/api/health` response body.
 * @returns `true` when `status.status === "ok"` and the database is connected.
 * @example
 * ```ts
 * import { isHealthy } from "@shadowcat/core";
 *
 * isHealthy({ status: "ok", db_connected: true });
 * ```
 */
export function isHealthy(status: HealthStatus): boolean {
  return status.status === "ok" && status.db_connected;
}

export { silentLogger, consoleLogger } from "./logger";
export type { Logger } from "./logger";
export { HookBus, STOP } from "./hooks";
export type { HookKind, HookDefinition, OnOptions, Handler, CoreHooks } from "./hooks";
export { ServiceRegistry } from "./services";
export { MiddlewareChain } from "./middleware";
export type { PipelineName, Middleware } from "./middleware";
export { ManifestSchema, parseManifest, declarationOf } from "./manifest";
export type {
  ModuleManifest,
  ModuleEngines,
  CapRequirement,
  HookDecl,
  ContractProvide,
  ContractDeclaration,
} from "./manifest";
export { ModuleRegistry } from "./modules";
export type { Module, ModuleContext, ModuleInfo } from "./modules";
export { loadModules } from "./loader";
export type { ImportFn, ModuleEntry, ModuleLoadFailure, ModuleLoadResult } from "./loader";
export { resolveCaps, canWritePath } from "./capabilities";
export { DocumentStore, setPointer, removePointer, getPointer, applyOperation } from "./store";
export type { Listener, ReadableDocuments } from "./store";
export { ContributionRegistry, PANEL_CONTRACT } from "./contributions";
export type { Contribution, Cardinality, PanelMeta, PanelBadge, DefaultPlacement, ZoneId, SheetMeta } from "./contributions";
export { reconcileTopology } from "./topology";
export { I18n } from "./i18n";
export type { Messages, I18nParams } from "./i18n";
export { OptimisticClient } from "./optimistic";
export { WsClient } from "./ws-client";
export type {
  WsClientOptions,
  WsClientHandlers,
  WireWelcome,
  SearchPage,
  PathResult,
  MoveSample,
  MoveVisionSample,
  MoveStream,
  SubscriptionHandle,
  SceneFrame,
  SceneSubscription,
  ChatSendOptions,
} from "./ws-client";
export { webSocketConnect } from "./transport";
export type { Transport, TransportHandlers, Connect } from "./transport";
export {
  parseServerMsg,
  DocumentSchema,
  CommandSchema,
  OperationSchema,
  ServerMsgSchema,
  CapabilityRequirementSchema,
  SearchHitSchema,
} from "./wire";
export type {
  ServerMsg,
  ClientMsg,
  WireDocument,
  WireCommand,
  WireOperation,
  WireFieldChange,
  WireScope,
  WireCapabilityGrants,
  WireCapabilityRequirement,
  WireContractProvide,
  WireContractDeclaration,
  WireSearchHit,
  WireActorOwnerRef,
  WireAudience,
  WirePermissionSet,
  WireMoveStreamSample,
  WireMoveStreamVisionSample,
} from "./wire";
export { AssetResolver } from "./assets";
export type { AssetOp } from "./assets";
export { listAssets, uploadAsset, replaceAsset, deleteAsset } from "./asset-rest";
export { listInstalledModules, getEnabledModules, setEnabledModules } from "./module-rest";
export type { InstalledModuleInfo } from "@shadowcat/types";
export { listUsers, createUser, deleteUser, listWorldMembers, createWorldInvite, listWorldInvites, revokeWorldInvite } from "./user-rest";
export type { ServerUser, WorldMember, MintedInvite, InviteEntry } from "./user-rest";
export { buildSceneDoc, buildTokenDoc, buildSceneEntityDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, resolveSceneSettings, resolveViewedScene, DEFAULT_GRADATION, buildLightGradationDoc, resolveGradation, SEED_VISION_MODES, buildVisionModesDoc, resolveVisionModes, buildLightDoc, buildRegionDoc, setRegionVisibility, DEFAULT_SCENE_BOUNDS, envelope, buildItemDoc, ITEM_DOC_TYPE, deterministicId } from "./scene-docs";
export type { SceneEngine, TokenEngine, ActorEngine, TokenOverrides, RenderVisual, AnimatedSource, FaceVisual, TokenVisual, Faction, FactionStance, FactionRegistryEngine, Condition, ConditionRegistryEngine, MovementRestriction, MovementModel, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsEngine, ResolvedSceneSettings, GradationBand, LightGradationEngine, VisionMode, VisionModesEngine, VisionAssignment, LightEngine, RegionShapeKind, RegionShape, RegionBehavior, RegionEngine, SceneDimensions, ItemSystem, DrawingEngine, DrawingShape, TemplateEngine, TemplateShape, Stroke, Fill, Grid, WallEngine, Seg } from "./scene-docs";
export { resolveTokenActor, effectiveOwner, ownerFloorApplies, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius, resolveTokenVisual, selectedFaceNamesFor } from "./actor";
export type { EffectiveActor, ConditionTarget, TokenBox } from "./actor";
export { SHEET_CONTRACT_PREFIX, SHEET_FALLBACK_CONTRACT, sheetContract, resolveDocRef, pickSheet, isDiceNotation } from "./sheets";
export type { SheetRef, SheetTarget } from "./sheets";
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, ChatSegmentSchema, ChatMessageEngineSchema, parseMessageEngine, isKnownSegment, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
export type { MessageKind, DieRecord, RollOutcome, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm } from "./chat-docs";
export { structuralDiff, deletePointer, deepEqual, merge3Tree, takeTemplate, isPlacementExcluded, merge3, restampSubtree, placementExclusions } from "./merge";
export type { Diff, Conflict, MergeBands, MergeBase, EmbeddedBaseChild, MergePlan } from "./merge";
export { snapshotBase, stampInstance, computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState } from "./templates";
export type { StampOpts, SyncState } from "./templates";
