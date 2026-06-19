import type { HealthStatus } from "@shadowcat/types";

/** Returns true when the server reports itself healthy with a live database. */
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
export { ManifestSchema, parseManifest } from "./manifest";
export type { ModuleManifest, CapRequirement, HookDecl } from "./manifest";
export { ModuleRegistry } from "./modules";
export type { Module, ModuleContext, ModuleInfo } from "./modules";
export { DocumentStore, setPointer, applyOperation } from "./store";
export type { Listener } from "./store";
export { OptimisticClient } from "./optimistic";
export { WsClient } from "./ws-client";
export type { WsClientOptions, WsClientHandlers } from "./ws-client";
export { webSocketConnect } from "./transport";
export type { Transport, TransportHandlers, Connect } from "./transport";
export {
  parseServerMsg,
  DocumentSchema,
  CommandSchema,
  OperationSchema,
  ServerMsgSchema,
} from "./wire";
export type {
  ServerMsg,
  ClientMsg,
  WireDocument,
  WireCommand,
  WireOperation,
  WireFieldChange,
  WireScope,
} from "./wire";
