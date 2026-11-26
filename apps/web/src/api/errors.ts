import { ApiError } from "@web/api/client";

/** What a screen shows for a failed request. */
export type Problem = {
  message: string;
  /** The request field the server rejected (`400 VALIDATION_ERROR`). */
  field?: string;
  /** Seconds to keep the submit button disabled (`429`). */
  retryAfterSecs?: number;
};

/** Turns any thrown value into a message for the user; servers' messages are shown as they are. */
export function describeError(error: unknown): Problem {
  if (!(error instanceof ApiError)) return { message: "Something went wrong. Try again." };
  return { message: error.message, field: error.field, retryAfterSecs: error.retryAfterSecs };
}
