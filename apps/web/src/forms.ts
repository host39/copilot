import type { z } from "zod";

/**
 * The first message for each top-level field of a failed parse, so a form shows the same rules
 * the server applies before sending.
 */
export function fieldErrors(error: z.ZodError): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const issue of error.issues) {
    const field = issue.path[0];
    if (typeof field === "string" && !(field in errors)) errors[field] = issue.message;
  }
  return errors;
}

/** The trimmed text of a form field (`""` when it is missing). */
export function text(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === "string" ? value.trim() : "";
}
