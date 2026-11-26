// Zod schemas for the API contracts. The Rust structs in `packages/contracts/src/` are the source
// of truth: `make contracts` regenerates the types in `ts/generated/`, and each schema below must
// satisfy its generated type, so drift fails `make type-check`.
//
// Identity DTOs come from lib-rustauth. `@rustauth/contracts` is pinned to the same commit as the
// `rustauth` crate (bump both together) and re-exported here, so `apps/web` imports only
// `@copilot/contracts`. Names defined in this file take precedence over the re-export: the shared
// envelopes, `errorResponseSchema`, and `REQUEST_ID_HEADER` match rustauth's (see
// `test/auth.test.ts`).
//
// Response schemas mirror what the server sends (every field, `null` for a missing value).
// Request schemas accept what the server accepts; the server stays the authority on rules and
// answers `400 VALIDATION_ERROR` with the failing `field`.

export * from "@rustauth/contracts";

import type { ApprovalMode } from "@contracts/generated/ApprovalMode";
import type { BuiltinTool } from "@contracts/generated/BuiltinTool";
import type { Capabilities } from "@contracts/generated/Capabilities";
import type { Capability } from "@contracts/generated/Capability";
import type { CapabilityLock } from "@contracts/generated/CapabilityLock";
import type { CapabilityOverrides } from "@contracts/generated/CapabilityOverrides";
import type { Capacity } from "@contracts/generated/Capacity";
import type { CapacityLevel } from "@contracts/generated/CapacityLevel";
import type { CreatedBy } from "@contracts/generated/CreatedBy";
import type { CreateProjectRequest } from "@contracts/generated/CreateProjectRequest";
import type { CreateProviderRequest } from "@contracts/generated/CreateProviderRequest";
import type { CreateScheduleRequest } from "@contracts/generated/CreateScheduleRequest";
import type { CreateToolServerRequest } from "@contracts/generated/CreateToolServerRequest";
import type { Credentials } from "@contracts/generated/Credentials";
import type { ErrorResponse } from "@contracts/generated/ErrorResponse";
import type { FileResponse } from "@contracts/generated/FileResponse";
import type { HealthResponse } from "@contracts/generated/HealthResponse";
import type { KeyState } from "@contracts/generated/KeyState";
import type { ModelResponse } from "@contracts/generated/ModelResponse";
import type { MoveFileRequest } from "@contracts/generated/MoveFileRequest";
import type { OnAsk } from "@contracts/generated/OnAsk";
import type { Preset } from "@contracts/generated/Preset";
import type { ProjectDetailsResponse } from "@contracts/generated/ProjectDetailsResponse";
import type { ProjectListQuery } from "@contracts/generated/ProjectListQuery";
import type { ProjectResponse } from "@contracts/generated/ProjectResponse";
import type { ProviderDetailsResponse } from "@contracts/generated/ProviderDetailsResponse";
import type { ProviderKind } from "@contracts/generated/ProviderKind";
import type { ProviderResponse } from "@contracts/generated/ProviderResponse";
import type { ProviderUsage } from "@contracts/generated/ProviderUsage";
import type { ReadyResponse } from "@contracts/generated/ReadyResponse";
import type { ReasoningLevel } from "@contracts/generated/ReasoningLevel";
import type { RefreshResponse } from "@contracts/generated/RefreshResponse";
import type { RootResponse } from "@contracts/generated/RootResponse";
import type { RunStatus } from "@contracts/generated/RunStatus";
import type { ScheduleListQuery } from "@contracts/generated/ScheduleListQuery";
import type { SchedulePreviewRequest } from "@contracts/generated/SchedulePreviewRequest";
import type { SchedulePreviewResponse } from "@contracts/generated/SchedulePreviewResponse";
import type { ScheduleResponse } from "@contracts/generated/ScheduleResponse";
import type { ScheduleRunResponse } from "@contracts/generated/ScheduleRunResponse";
import type { ServerTool } from "@contracts/generated/ServerTool";
import type { SettingsResponse } from "@contracts/generated/SettingsResponse";
import type { ToolChanges } from "@contracts/generated/ToolChanges";
import type { ToolMode } from "@contracts/generated/ToolMode";
import type { ToolServerResponse } from "@contracts/generated/ToolServerResponse";
import type { ToolTestResponse } from "@contracts/generated/ToolTestResponse";
import type { UpdateFileRequest } from "@contracts/generated/UpdateFileRequest";
import type { UpdateModelRequest } from "@contracts/generated/UpdateModelRequest";
import type { UpdateProjectRequest } from "@contracts/generated/UpdateProjectRequest";
import type { UpdateProviderRequest } from "@contracts/generated/UpdateProviderRequest";
import type { UpdateScheduleRequest } from "@contracts/generated/UpdateScheduleRequest";
import type { UpdateSettingsRequest } from "@contracts/generated/UpdateSettingsRequest";
import type { UpdateToolServerRequest } from "@contracts/generated/UpdateToolServerRequest";
import { z } from "zod";

export type { ApprovalMode } from "@contracts/generated/ApprovalMode";
export type { BuiltinTool } from "@contracts/generated/BuiltinTool";
export type { Capabilities } from "@contracts/generated/Capabilities";
export type { Capability } from "@contracts/generated/Capability";
export type { CapabilityLock } from "@contracts/generated/CapabilityLock";
export type { CapabilityOverrides } from "@contracts/generated/CapabilityOverrides";
export type { Capacity } from "@contracts/generated/Capacity";
export type { CapacityLevel } from "@contracts/generated/CapacityLevel";
export type { CreatedBy } from "@contracts/generated/CreatedBy";
export type { CreateProjectRequest } from "@contracts/generated/CreateProjectRequest";
export type { CreateProviderRequest } from "@contracts/generated/CreateProviderRequest";
export type { CreateScheduleRequest } from "@contracts/generated/CreateScheduleRequest";
export type { CreateToolServerRequest } from "@contracts/generated/CreateToolServerRequest";
export type { Credentials } from "@contracts/generated/Credentials";
export type { DetailsResponse } from "@contracts/generated/DetailsResponse";
export type { ErrorResponse } from "@contracts/generated/ErrorResponse";
export type { FileResponse } from "@contracts/generated/FileResponse";
export type { HealthResponse } from "@contracts/generated/HealthResponse";
export type { KeyState } from "@contracts/generated/KeyState";
export type { ListResponse } from "@contracts/generated/ListResponse";
export type { ModelResponse } from "@contracts/generated/ModelResponse";
export type { MoveFileRequest } from "@contracts/generated/MoveFileRequest";
export type { OnAsk } from "@contracts/generated/OnAsk";
export type { Preset } from "@contracts/generated/Preset";
export type { ProjectDetailsResponse } from "@contracts/generated/ProjectDetailsResponse";
export type { ProjectListQuery } from "@contracts/generated/ProjectListQuery";
export type { ProjectResponse } from "@contracts/generated/ProjectResponse";
export type { ProviderDetailsResponse } from "@contracts/generated/ProviderDetailsResponse";
export type { ProviderKind } from "@contracts/generated/ProviderKind";
export type { ProviderResponse } from "@contracts/generated/ProviderResponse";
export type { ProviderUsage } from "@contracts/generated/ProviderUsage";
export type { ReadyResponse } from "@contracts/generated/ReadyResponse";
export type { ReasoningLevel } from "@contracts/generated/ReasoningLevel";
export type { RefreshResponse } from "@contracts/generated/RefreshResponse";
export type { RootResponse } from "@contracts/generated/RootResponse";
export type { RunStatus } from "@contracts/generated/RunStatus";
export type { ScheduleListQuery } from "@contracts/generated/ScheduleListQuery";
export type { SchedulePreviewRequest } from "@contracts/generated/SchedulePreviewRequest";
export type { SchedulePreviewResponse } from "@contracts/generated/SchedulePreviewResponse";
export type { ScheduleResponse } from "@contracts/generated/ScheduleResponse";
export type { ScheduleRunResponse } from "@contracts/generated/ScheduleRunResponse";
export type { ServerTool } from "@contracts/generated/ServerTool";
export type { SettingsResponse } from "@contracts/generated/SettingsResponse";
export type { ToolChanges } from "@contracts/generated/ToolChanges";
export type { ToolMode } from "@contracts/generated/ToolMode";
export type { ToolServerResponse } from "@contracts/generated/ToolServerResponse";
export type { ToolTestResponse } from "@contracts/generated/ToolTestResponse";
export type { UpdateFileRequest } from "@contracts/generated/UpdateFileRequest";
export type { UpdateModelRequest } from "@contracts/generated/UpdateModelRequest";
export type { UpdateProjectRequest } from "@contracts/generated/UpdateProjectRequest";
export type { UpdateProviderRequest } from "@contracts/generated/UpdateProviderRequest";
export type { UpdateScheduleRequest } from "@contracts/generated/UpdateScheduleRequest";
export type { UpdateSettingsRequest } from "@contracts/generated/UpdateSettingsRequest";
export type { UpdateToolServerRequest } from "@contracts/generated/UpdateToolServerRequest";

export const REQUEST_ID_HEADER = "X-Request-Id";

/** The multipart field of `POST /projects/{id}/files` and `POST /chats/{id}/files`. */
export const UPLOAD_FIELD = "file";

// Envelopes and health

export const rootResponseSchema = z.object({
  status: z.string(),
  environment: z.string(),
}) satisfies z.ZodType<RootResponse>;

export const healthResponseSchema = z.object({
  status: z.string(),
}) satisfies z.ZodType<HealthResponse>;

export const readyResponseSchema = z.object({
  status: z.string(),
  database: z.string(),
}) satisfies z.ZodType<ReadyResponse>;

export const errorResponseSchema = z.object({
  error: z.string(),
  code: z.string().optional(),
  field: z.string().optional(),
  details: z.record(z.string(), z.json()).optional(),
  requestId: z.string().optional(),
}) satisfies z.ZodType<ErrorResponse>;

/** `{ items, total }` list envelope around `item`. */
export function listResponseSchema<T extends z.ZodType>(item: T) {
  return z.object({
    items: z.array(item),
    total: z.number().int().nonnegative(),
  });
}

/** `{ data }` details envelope around `data`. Pass `schema.nullable()` when absence is valid. */
export function detailsResponseSchema<T extends z.ZodType>(data: T) {
  return z.object({ data });
}

// Shared values

const id = z.uuid();
const timestamp = z.iso.datetime({ offset: true });
const count = z.number().int().nonnegative();

export const reasoningLevelSchema = z.enum([
  "off",
  "low",
  "medium",
  "high",
]) satisfies z.ZodType<ReasoningLevel>;

export const builtinToolSchema = z.enum(["schedules"]) satisfies z.ZodType<BuiltinTool>;

export const keyStateSchema = z.enum([
  "missing",
  "set",
  "unreadable",
]) satisfies z.ZodType<KeyState>;

// Settings

export const settingsResponseSchema = z.object({
  instructions: z.string(),
}) satisfies z.ZodType<SettingsResponse>;

export const updateSettingsRequestSchema = z.object({
  instructions: z.string().optional(),
}) satisfies z.ZodType<UpdateSettingsRequest>;

// Providers

export const providerKindSchema = z.enum(["openai", "anthropic"]) satisfies z.ZodType<ProviderKind>;

export const presetSchema = z.enum([
  "openai",
  "anthropic",
  "gemini",
  "openrouter",
  "ollama",
  "custom",
]) satisfies z.ZodType<Preset>;

export const providerResponseSchema = z.object({
  id,
  kind: providerKindSchema,
  preset: presetSchema,
  name: z.string(),
  baseUrl: z.string(),
  enabled: z.boolean(),
  key: keyStateSchema,
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ProviderResponse>;

export const providerUsageSchema = z.object({
  chats: count,
  projects: count,
  schedules: count,
}) satisfies z.ZodType<ProviderUsage>;

export const providerDetailsResponseSchema = providerResponseSchema.extend({
  usage: providerUsageSchema,
}) satisfies z.ZodType<ProviderDetailsResponse>;

export const createProviderRequestSchema = z.object({
  kind: providerKindSchema,
  preset: presetSchema,
  name: z.string(),
  baseUrl: z.string().optional(),
  apiKey: z.string().optional(),
}) satisfies z.ZodType<CreateProviderRequest>;

export const updateProviderRequestSchema = z.object({
  name: z.string().optional(),
  baseUrl: z.string().optional(),
  apiKey: z.string().optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateProviderRequest>;

export const refreshResponseSchema = z.object({
  added: count,
  updated: count,
  unavailable: count,
  enabled: count,
}) satisfies z.ZodType<RefreshResponse>;

// Models

export const capabilitySchema = z.enum([
  "vision",
  "pdf",
  "tools",
  "audioIn",
  "audioOut",
  "reasoning",
]) satisfies z.ZodType<Capability>;

export const capabilitiesSchema = z.object({
  vision: z.boolean(),
  pdf: z.boolean(),
  tools: z.boolean(),
  audioIn: z.boolean(),
  audioOut: z.boolean(),
  reasoning: z.boolean(),
  reasoningLevels: z.array(reasoningLevelSchema),
  defaultReasoning: reasoningLevelSchema.nullable(),
}) satisfies z.ZodType<Capabilities>;

export const capabilityOverridesSchema = z.object({
  vision: z.boolean().nullable(),
  pdf: z.boolean().nullable(),
  tools: z.boolean().nullable(),
  audioIn: z.boolean().nullable(),
  audioOut: z.boolean().nullable(),
  reasoning: z.boolean().nullable(),
}) satisfies z.ZodType<CapabilityOverrides>;

export const capabilityLockSchema = z.object({
  capability: capabilitySchema,
  reason: z.string(),
}) satisfies z.ZodType<CapabilityLock>;

export const modelResponseSchema = z.object({
  id,
  providerId: id,
  providerName: z.string(),
  kind: providerKindSchema,
  preset: presetSchema,
  remoteId: z.string(),
  displayName: z.string(),
  enabled: z.boolean(),
  available: z.boolean(),
  contextWindow: count.nullable(),
  maxOutputTokens: count.nullable(),
  capabilities: capabilitiesSchema,
  detected: capabilitiesSchema,
  overrides: capabilityOverridesSchema,
  locks: z.array(capabilityLockSchema),
  reasoningOffWithTools: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ModelResponse>;

export const updateModelRequestSchema = z.object({
  enabled: z.boolean().optional(),
  displayName: z.string().optional(),
  capabilityOverrides: capabilityOverridesSchema.optional(),
  contextWindow: count.nullable().optional(),
  maxOutputTokens: count.nullable().optional(),
}) satisfies z.ZodType<UpdateModelRequest>;

// Files

export const fileResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  chatId: id.nullable(),
  name: z.string(),
  mediaType: z.string(),
  sizeBytes: count,
  tokenEstimate: count,
  pageCount: count.nullable(),
  included: z.boolean(),
  createdAt: timestamp,
}) satisfies z.ZodType<FileResponse>;

export const updateFileRequestSchema = z.object({
  name: z.string().optional(),
  included: z.boolean().optional(),
}) satisfies z.ZodType<UpdateFileRequest>;

export const moveFileRequestSchema = z.object({
  projectId: id,
}) satisfies z.ZodType<MoveFileRequest>;

export const capacityLevelSchema = z.enum([
  "unknown",
  "fits",
  "warn",
  "over",
]) satisfies z.ZodType<CapacityLevel>;

export const capacitySchema = z.object({
  used: count,
  window: count.nullable(),
  level: capacityLevelSchema,
}) satisfies z.ZodType<Capacity>;

// Projects

export const projectResponseSchema = z.object({
  id,
  name: z.string(),
  icon: z.string(),
  instructions: z.string(),
  defaultModelId: id.nullable(),
  reasoning: reasoningLevelSchema.nullable(),
  builtinTools: z.array(builtinToolSchema),
  toolServerIds: z.array(id),
  pinned: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ProjectResponse>;

export const projectDetailsResponseSchema = projectResponseSchema.extend({
  capacity: capacitySchema,
}) satisfies z.ZodType<ProjectDetailsResponse>;

export const projectListQuerySchema = z.object({
  pinned: z.boolean().optional(),
  limit: count.optional(),
}) satisfies z.ZodType<ProjectListQuery>;

export const createProjectRequestSchema = z.object({
  name: z.string(),
  icon: z.string().optional(),
  instructions: z.string().optional(),
  defaultModelId: id.optional(),
  reasoning: reasoningLevelSchema.optional(),
  builtinTools: z.array(builtinToolSchema).optional(),
  toolServerIds: z.array(id).optional(),
  pinned: z.boolean().optional(),
}) satisfies z.ZodType<CreateProjectRequest>;

export const updateProjectRequestSchema = z.object({
  name: z.string().optional(),
  icon: z.string().optional(),
  instructions: z.string().optional(),
  defaultModelId: id.nullable().optional(),
  reasoning: reasoningLevelSchema.nullable().optional(),
  builtinTools: z.array(builtinToolSchema).optional(),
  toolServerIds: z.array(id).optional(),
  pinned: z.boolean().optional(),
}) satisfies z.ZodType<UpdateProjectRequest>;

// Tool servers

export const approvalModeSchema = z.enum(["ask", "auto"]) satisfies z.ZodType<ApprovalMode>;

export const toolModeSchema = z.enum(["off", "ask", "auto"]) satisfies z.ZodType<ToolMode>;

export const serverToolSchema = z.object({
  name: z.string(),
  title: z.string().nullable(),
  description: z.string(),
  inputSchema: z.json(),
  readOnly: z.boolean(),
}) satisfies z.ZodType<ServerTool>;

export const toolServerResponseSchema = z.object({
  id,
  name: z.string(),
  slug: z.string(),
  url: z.string(),
  approval: approvalModeSchema,
  toolModes: z.record(z.string(), toolModeSchema),
  enabled: z.boolean(),
  credentials: keyStateSchema,
  protocolVersion: z.string().nullable(),
  tools: z.array(serverToolSchema).nullable(),
  toolsFetchedAt: timestamp.nullable(),
  reviewRequired: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ToolServerResponse>;

export const createToolServerRequestSchema = z.object({
  name: z.string(),
  slug: z.string().optional(),
  url: z.string(),
  bearer: z.string().optional(),
  headers: z.record(z.string(), z.string()).default({}),
  approval: approvalModeSchema.optional(),
}) satisfies z.ZodType<CreateToolServerRequest>;

export const credentialsSchema = z.object({
  bearer: z.string().optional(),
  headers: z.record(z.string(), z.string()).default({}),
}) satisfies z.ZodType<Credentials>;

export const updateToolServerRequestSchema = z.object({
  name: z.string().optional(),
  url: z.string().optional(),
  credentials: credentialsSchema.nullable().optional(),
  approval: approvalModeSchema.optional(),
  toolModes: z.record(z.string(), toolModeSchema).optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateToolServerRequest>;

export const toolChangesSchema = z.object({
  added: z.array(z.string()),
  removed: z.array(z.string()),
  changed: z.array(z.string()),
}) satisfies z.ZodType<ToolChanges>;

export const toolTestResponseSchema = z.object({
  server: toolServerResponseSchema,
  changes: toolChangesSchema,
}) satisfies z.ZodType<ToolTestResponse>;

// Schedules

export const onAskSchema = z.enum(["deny", "pause"]) satisfies z.ZodType<OnAsk>;

export const createdBySchema = z.enum(["user", "assistant"]) satisfies z.ZodType<CreatedBy>;

export const runStatusSchema = z.enum([
  "ok",
  "error",
  "awaiting_approval",
]) satisfies z.ZodType<RunStatus>;

export const scheduleResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  modelId: id.nullable(),
  sourceChatId: id.nullable(),
  name: z.string(),
  cron: z.string(),
  timezone: z.string(),
  description: z.string(),
  prompt: z.string(),
  instructions: z.string(),
  reasoning: reasoningLevelSchema.nullable(),
  onAsk: onAskSchema,
  createdBy: createdBySchema,
  enabled: z.boolean(),
  nextRunAt: timestamp.nullable(),
  lastRunAt: timestamp.nullable(),
  lastStatus: runStatusSchema.nullable(),
  lastError: z.string().nullable(),
  consecutiveFailures: count,
  disabledReason: z.string().nullable(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ScheduleResponse>;

export const scheduleListQuerySchema = z.object({
  projectId: id.optional(),
}) satisfies z.ZodType<ScheduleListQuery>;

export const createScheduleRequestSchema = z.object({
  name: z.string(),
  cron: z.string(),
  timezone: z.string(),
  prompt: z.string(),
  projectId: id.optional(),
  modelId: id.optional(),
  instructions: z.string().optional(),
  reasoning: reasoningLevelSchema.optional(),
  onAsk: onAskSchema.optional(),
}) satisfies z.ZodType<CreateScheduleRequest>;

export const updateScheduleRequestSchema = z.object({
  name: z.string().optional(),
  cron: z.string().optional(),
  timezone: z.string().optional(),
  prompt: z.string().optional(),
  instructions: z.string().optional(),
  modelId: id.optional(),
  reasoning: reasoningLevelSchema.nullable().optional(),
  onAsk: onAskSchema.optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateScheduleRequest>;

export const schedulePreviewRequestSchema = z.object({
  cron: z.string(),
  timezone: z.string(),
}) satisfies z.ZodType<SchedulePreviewRequest>;

export const schedulePreviewResponseSchema = z.object({
  description: z.string(),
  timezone: z.string(),
  nextRuns: z.array(timestamp),
}) satisfies z.ZodType<SchedulePreviewResponse>;

export const scheduleRunResponseSchema = z.object({
  chatId: id,
}) satisfies z.ZodType<ScheduleRunResponse>;

// Route bodies: the envelope of each success response.

/** `GET` and `PATCH /settings` */
export const settingsBodySchema = detailsResponseSchema(settingsResponseSchema);
/** `GET /providers` */
export const providerListBodySchema = listResponseSchema(providerResponseSchema);
/** `POST /providers`, `PATCH /providers/{id}` */
export const providerBodySchema = detailsResponseSchema(providerResponseSchema);
/** `GET /providers/{id}` */
export const providerDetailsBodySchema = detailsResponseSchema(providerDetailsResponseSchema);
/** `POST /providers/{id}/refresh` */
export const refreshBodySchema = detailsResponseSchema(refreshResponseSchema);
/** `GET /models`, `GET /providers/{id}/models` */
export const modelListBodySchema = listResponseSchema(modelResponseSchema);
/** `PATCH /models/{id}` */
export const modelBodySchema = detailsResponseSchema(modelResponseSchema);
/** `GET /projects/{id}/files` */
export const fileListBodySchema = listResponseSchema(fileResponseSchema);
/** `POST /projects/{id}/files`, `POST /chats/{id}/files`, `PATCH /files/{id}`, `POST /files/{id}/move` */
export const fileBodySchema = detailsResponseSchema(fileResponseSchema);
/** `GET /projects` */
export const projectListBodySchema = listResponseSchema(projectResponseSchema);
/** `POST /projects`, `PATCH /projects/{id}` */
export const projectBodySchema = detailsResponseSchema(projectResponseSchema);
/** `GET /projects/{id}` */
export const projectDetailsBodySchema = detailsResponseSchema(projectDetailsResponseSchema);
/** `GET /tool-servers` */
export const toolServerListBodySchema = listResponseSchema(toolServerResponseSchema);
/** `GET`, `PATCH /tool-servers/{id}`, `POST /tool-servers`, `POST /tool-servers/{id}/review` */
export const toolServerBodySchema = detailsResponseSchema(toolServerResponseSchema);
/** `POST /tool-servers/{id}/test` */
export const toolTestBodySchema = detailsResponseSchema(toolTestResponseSchema);
/** `GET /schedules` */
export const scheduleListBodySchema = listResponseSchema(scheduleResponseSchema);
/** `GET`, `PATCH /schedules/{id}`, `POST /schedules` */
export const scheduleBodySchema = detailsResponseSchema(scheduleResponseSchema);
/** `POST /schedules/preview` */
export const schedulePreviewBodySchema = detailsResponseSchema(schedulePreviewResponseSchema);
/** `POST /schedules/{id}/run` */
export const scheduleRunBodySchema = detailsResponseSchema(scheduleRunResponseSchema);
