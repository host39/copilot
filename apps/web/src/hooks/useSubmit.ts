import { describeError } from "@web/api/errors";
import { fieldErrors } from "@web/forms";
import { useCountdown } from "@web/hooks/useCountdown";
import { useState } from "react";
import type { z } from "zod";

/**
 * A form's submit flow: checks the input with the request schema (the server's own rules) before
 * sending, then shows the server's answer. A rejected field goes next to its input, any other
 * failure above the form, and a `429` disables the button for the wait it names.
 */
export function useSubmit<S extends z.ZodType, R>(
  schema: S,
  send: (body: z.output<S>) => Promise<R>,
  done: (result: R) => void
) {
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [alert, setAlert] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [wait, startWait] = useCountdown();

  async function submit(input: unknown): Promise<void> {
    const parsed = schema.safeParse(input);
    setAlert(undefined);
    if (!parsed.success) {
      setErrors(fieldErrors(parsed.error));
      return;
    }
    setErrors({});
    setBusy(true);
    let result: R;
    try {
      result = await send(parsed.data);
    } catch (error) {
      const problem = describeError(error);
      if (problem.field) setErrors({ [problem.field]: problem.message });
      else setAlert(problem.message);
      startWait(problem.retryAfterSecs);
      setBusy(false);
      return;
    }
    setBusy(false);
    done(result);
  }

  return { submit, errors, alert, busy, wait };
}
