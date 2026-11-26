// Zod schemas for the API contracts. The Rust structs in `packages/contracts/src/` are the source
// of truth: `make contracts` regenerates the types in `ts/generated/`, and each schema below must
// satisfy its generated type, so drift fails `make type-check`.

import type { ErrorResponse } from "@contracts/generated/ErrorResponse";
import type { HealthResponse } from "@contracts/generated/HealthResponse";
import type { ReadyResponse } from "@contracts/generated/ReadyResponse";
import type { RootResponse } from "@contracts/generated/RootResponse";
import { z } from "zod";

export type { DetailsResponse } from "@contracts/generated/DetailsResponse";
export type { ErrorResponse } from "@contracts/generated/ErrorResponse";
export type { HealthResponse } from "@contracts/generated/HealthResponse";
export type { ListResponse } from "@contracts/generated/ListResponse";
export type { ReadyResponse } from "@contracts/generated/ReadyResponse";
export type { RootResponse } from "@contracts/generated/RootResponse";

export const REQUEST_ID_HEADER = "X-Request-Id";

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
