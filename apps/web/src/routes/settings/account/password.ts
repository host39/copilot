import { changePasswordRequestSchema } from "@copilot/contracts";
import { z } from "zod";

/**
 * The change-password form: rustauth's rules for the new password, a required current one, and
 * a repeat that must match. It parses to the request body (`current`, `new`).
 */
export const passwordSchema = changePasswordRequestSchema
  .extend({
    current: z.string().min(1, "Enter your current password."),
    confirm: z.string(),
  })
  .refine((input) => input.new === input.confirm, {
    message: "The passwords do not match.",
    path: ["confirm"],
  })
  .transform(({ current, new: next }) => ({ current, new: next }));
