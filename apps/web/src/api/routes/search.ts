// Search (`GET /search`): chat titles and message text.

import { type SearchHitResponse, searchListBodySchema } from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

/** Matches for `q`, best first; a title match has no `messageId`. */
export async function search(
  q: string,
  limit?: number,
  signal?: AbortSignal
): Promise<SearchHitResponse[]> {
  return (await apiFetch(API.search, searchListBodySchema, { query: { q, limit }, signal })).items;
}
