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
export { resolveCaps, canWritePath, canCreateDoc } from "./capabilities";
export { DocumentStore, setPointer, removePointer, getPointer, applyOperation } from "./store";
export type { Listener, ReadableDocuments } from "./store";
export { ContributionRegistry, PANEL_CONTRACT, SYSTEM_CONTRACT } from "./contributions";
export type { Contribution, Cardinality, PanelMeta, PanelBadge, DefaultPlacement, ZoneId, SheetMeta } from "./contributions";
export { reconcileTopology } from "./topology";
export { I18n } from "./i18n";
export type { Messages, I18nParams, AddMessagesOptions } from "./i18n";
export { NotificationCenter } from "./notifications";
export type { NotificationLevel, Notification, NotificationListener } from "./notifications";
export { OptimisticClient } from "./optimistic";
export { WsClient, MergeIntentError } from "./ws-client";
export type {
  WsClientOptions,
  WsClientHandlers,
  WsTimeoutOptions,
  WireWelcome,
  SearchPage,
  PathResult,
  MoveSample,
  MoveVisionSample,
  MoveLightSample,
  MoveStream,
  SubscriptionHandle,
  SceneFrame,
  SceneSubscription,
  ChatSendOptions,
  DrawTableOptions,
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
  parseCombats,
  EMPTY_COMBATS,
  CombatsPayloadSchema,
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
  WireRoleCapabilities,
  WireCapabilityRequirement,
  WireContractProvide,
  WireContractDeclaration,
  WireSearchHit,
  WireActorOwnerRef,
  WireAudience,
  WirePermissionSet,
  WireMoveStreamSample,
  WireMoveStreamVisionSample,
  WireRecalcOp,
  WireCombatRollEntry,
  WireResourceOp,
  CombatsView,
  CombatView,
  CombatantView,
  ResolvedResourceView,
  WireMergeConflict,
  WireMergePullStatus,
  WirePushInstanceStatus,
  WirePushInstanceOutcome,
  WireMergeOutcome,
  WireMergeErrorKind,
} from "./wire";
export { AssetResolver } from "./assets";
export type {
  AssetOp,
  AssetVariant,
  AssetChangedNotice,
  ListingInvalidatedHandler,
} from "./assets";
export {
  listAssets,
  getAssetMeta,
  uploadAsset,
  replaceAsset,
  deleteAsset,
  queryAssets,
  patchAsset,
  bulkPatchAssets,
  reconvertAsset,
  originalUrl,
  restErrorText,
  deleteAssetFolder,
} from "./asset-rest";
export type { AssetQuery } from "./asset-rest";
export { AssetMetaCache } from "./asset-meta";
export { resolveVfxSource } from "./vfx";
export type { VfxPlayRequest, VfxOneShotRequest, ResolvedVfxSource } from "./vfx";
export { startChunkedUpload, ChunkedUploadError, CHUNK_THRESHOLD_BYTES } from "./asset-upload";
export type { ChunkedUploadOptions } from "./asset-upload";
export { listInstalledModules, getEnabledModules, setEnabledModules } from "./module-rest";
export type { InstalledModuleInfo } from "@shadowcat/types";
export { listUsers, createUser, deleteUser, listWorldMembers, createWorldInvite, listWorldInvites, revokeWorldInvite } from "./user-rest";
export type { ServerUser, WorldMember, MintedInvite, InviteEntry } from "./user-rest";
export { ACTOR_DOC_TYPE, buildSceneDoc, buildTokenDoc, buildSceneEntityDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, resolveSceneSettings, resolveViewedScene, DEFAULT_GRADATION, buildLightGradationDoc, resolveGradation, SEED_VISION_MODES, buildVisionModesDoc, resolveVisionModes, buildLightDoc, DEFAULT_LIGHT_EMISSION, buildRegionDoc, setRegionVisibility, DEFAULT_SCENE_BOUNDS, envelope, buildItemDoc, ITEM_DOC_TYPE, deterministicId, COMBAT_DOC_TYPE, COMBATANT_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE, EFFECT_DOC_TYPE, COMBAT_HISTORY_DOC_TYPE, buildCombatDoc, buildCombatantDoc, newCombatEngine, ENGINE_COMBAT_DEFAULTS, buildResourceRegistryDoc, buildEffectDoc, buildCombatHistoryDoc, SYSTEM_DEFAULTS_DOC_TYPE, buildSystemDefaultsDoc, resolveSettingProvenance, AUTHOR_CAPS, grantAuthor } from "./scene-docs";
export type { SceneEngine, TokenEngine, ActorEngine, TokenOverrides, RenderVisual, AnimatedSource, GeneratedCrop, GeneratedBorder, GeneratedBackground, FaceVisual, TokenVisual, AuraEmission, SoundEmission, VfxEmission, VfxAnchor, Faction, FactionStance, FactionRegistryEngine, Condition, ConditionFx, ConditionRegistryEngine, MovementRestriction, MovementModel, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsEngine, ResolvedSceneSettings, GradationBand, LightGradationEngine, VisionMode, Perception, VisionModesEngine, VisionAssignment, LightEngine, LightEmission, Falloff, FalloffCurve, RegionShapeKind, RegionShape, RegionBehavior, RegionEngine, RegionTrigger, TriggerEvent, TriggerEffect, NoticeAudience, SceneDimensions, ItemSystem, DrawingEngine, DrawingShape, TemplateEngine, TemplateShape, Stroke, Fill, Grid, WallEngine, Seg, CombatEngine, CombatantEngine, CombatantKind, CombatantResource, CombatDefaults, MovementRules, Interpretation, Enforcement, TurnControl, ResourceRegistryEngine, Resource, ResourceBinding, Recovery, Formula, EffectEngine, Duration, DurationUnit, ExpiryPoint, EffectLifecycle, EffectLifecycleDefaults, EffectSnapshot, CapturedCombatant, TurnRecord, CombatHistoryEngine, CombatantDocOptions, SystemDefaultsEngine, SceneDefaultsOverlay, PathfindingOverlay, AnimationOverlay, SettingSource, SettingPath } from "./scene-docs";
export { resolveTokenActor, effectiveOwner, ownerFloorApplies, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, resolveTokenVisual, selectedFaceNamesFor } from "./actor";
export type { EffectiveActor, ConditionTarget, TokenBox } from "./actor";
export { parseFootprints, EMPTY_FOOTPRINTS } from "./footprints";
export type { FootprintExtent, FootprintLookup } from "./footprints";
export { COMBAT_SERVICE, CombatController, CombatClientError } from "./combat";
export type {
  CombatApi,
  CombatControllerDeps,
  CombatAffordances,
  CreateCombatOptions,
  NewCombatant,
  NewEvent,
  WorldRole,
} from "./combat";
export {
  COMBAT_HOOK_VERSION,
  defineCombatHooks,
  deriveCombatHookEvents,
  commandTouchesCombat,
  CombatHookEmitter,
} from "./combat-hooks";
export type { CombatHookEvent, CombatTurnEvent } from "./combat-hooks";
export { SHEET_CONTRACT_PREFIX, SHEET_FALLBACK_CONTRACT, sheetContract, resolveDocRef, pickSheet, isDiceNotation } from "./sheets";
export type { SheetRef, SheetTarget } from "./sheets";
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, DocLinkTargetSchema, ChatSegmentSchema, SegmentListSchema, ChatMessageEngineSchema, WireDieKindSchema, WireRawRollSchema, RecalcHistoryEntrySchema, parseMessageEngine, isKnownSegment, baseRollDice, numericBounds, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc, firstChannel } from "./chat-docs";
export type { MessageKind, DieRecord, RollOutcome, DocLinkTarget, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm, WireDieKind, WireRawRoll, RecalcHistoryEntry, TableDrawSegment, DrawnRow } from "./chat-docs";
export { buildMoveOp } from "./move-op";
export { buildUpdate } from "./update-op";
export type { FieldEdit } from "./update-op";
export { TABLE_DOC_TYPE, buildTableDoc } from "./table-docs";
export type { TableEngine, DrawRule, TableRow, RowRange, TableEntry, BuildTableDocOptions } from "./table-docs";
export { NOTE_DOC_TYPE, buildNoteDoc, parseNoteBody } from "./note-docs";
export type { NoteEngine, BuildNoteDocOptions } from "./note-docs";
export { structuralDiff, deepEqual, isPlacementExcluded, restampSubtree, placementExclusions, isMergeableBandPointer, normalizeBase } from "./merge";
export type { Diff, MergeBase, EmbeddedBaseChild } from "./merge";
export { snapshotBase, stampInstance, findInstances, syncState } from "./templates";
export type { StampOpts, SyncState } from "./templates";
