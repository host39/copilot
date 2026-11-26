import { type HealthResponse, healthResponseSchema } from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

export async function fetchHealth(): Promise<HealthResponse> {
  return apiFetch(API.health, healthResponseSchema);
}
