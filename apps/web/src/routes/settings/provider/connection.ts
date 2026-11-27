// The Connection form of a provider's Settings page: it sends only what changed, and a key only
// when the user typed one (a blank field keeps the stored key).

import {
  type ProviderResponse,
  providerApiSchema,
  type UpdateProviderRequest,
} from "@copilot/contracts";
import { isHttp } from "@web/forms";
import { NAME_MAX_CHARS } from "@web/routes/settings/providers/presets";
import { z } from "zod";

/** Whether two URLs share scheme, host, and port (the server keeps the key only then). */
export function sameOrigin(a: string, b: string): boolean {
  try {
    return new URL(a).origin === new URL(b).origin;
  } catch {
    return false;
  }
}

/**
 * Whether the connection's page offers the OpenAI API choice: OpenAI and OpenAI-compatible
 * connections (the server refuses Responses for the other presets).
 */
export function choosesApi(provider: ProviderResponse): boolean {
  return (
    provider.kind === "openai" && (provider.preset === "openai" || provider.preset === "custom")
  );
}

/** The Connection form of `provider`; parses to a `PATCH /providers/{id}` body. */
export function connectionSchema(provider: ProviderResponse) {
  return z
    .object({
      name: z
        .string()
        .trim()
        .min(1, "Enter a name.")
        .refine(
          (value) => Array.from(value).length <= NAME_MAX_CHARS,
          "Use at most 100 characters."
        ),
      baseUrl: z.string().trim().refine(isHttp, "Enter a full http:// or https:// address."),
      apiKey: z.string().trim(),
      api: providerApiSchema.optional(),
    })
    .transform((input): UpdateProviderRequest => {
      const body: UpdateProviderRequest = {};
      if (input.name !== provider.name) body.name = input.name;
      if (input.baseUrl !== provider.baseUrl) body.baseUrl = input.baseUrl;
      if (input.apiKey !== "") body.apiKey = input.apiKey;
      if (input.api !== undefined && input.api !== provider.api && choosesApi(provider)) {
        body.api = input.api;
      }
      return body;
    });
}

/**
 * Whether saving `body` drops the stored key: the address moves to another origin and no new key
 * comes with it.
 */
export function dropsKey(provider: ProviderResponse, body: UpdateProviderRequest): boolean {
  return (
    provider.key !== "missing" &&
    body.apiKey === undefined &&
    body.baseUrl !== undefined &&
    !sameOrigin(body.baseUrl, provider.baseUrl)
  );
}
