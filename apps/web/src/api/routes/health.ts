import { type HealthResponse, healthResponseSchema } from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /health` of the configured server, or of `server` (the Connect screen's check). */
export async function fetchHealth(server?: string): Promise<HealthResponse> {
  return apiFetch(API.health, healthResponseSchema, { root: true, server });
}
