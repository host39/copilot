import { errorResponseSchema, REQUEST_ID_HEADER } from "@copilot/contracts";
import { API_URL } from "@web/config";
import { log } from "@web/logging";

type ResponseSchema<T> = {
  parse: (data: unknown) => T;
};

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly requestId?: string
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function readError(res: Response): Promise<ApiError> {
  const requestId = res.headers.get(REQUEST_ID_HEADER) ?? undefined;
  const parsed = errorResponseSchema.safeParse(await res.json().catch(() => undefined));
  const message = parsed.success ? parsed.data.error : `Request failed with status ${res.status}`;
  return new ApiError(message, res.status, parsed.data?.code, requestId);
}

export async function apiFetch<T>(path: string, schema?: ResponseSchema<T>): Promise<T> {
  const res = await fetch(`${API_URL}${path}`, {
    headers: { [REQUEST_ID_HEADER]: crypto.randomUUID() },
  });

  if (!res.ok) {
    const error = await readError(res);
    log("warn", "web.api.request.failed", {
      path,
      status: error.status,
      code: error.code,
      requestId: error.requestId,
    });
    throw error;
  }

  const data: unknown = await res.json();
  return schema ? schema.parse(data) : (data as T);
}
