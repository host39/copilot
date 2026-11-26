import Button from "@web/components/Button/Button";
import type { ReactNode } from "react";

type SubmitButtonProps = {
  busy: boolean;
  /** Seconds left of a `429` wait. */
  wait: number;
  children: ReactNode;
};

/** The full-width submit button of the account forms, disabled while a request runs or a wait lasts. */
export function SubmitButton({ busy, wait, children }: SubmitButtonProps) {
  return (
    <Button type="submit" busy={busy} disabled={wait > 0} className="w-full">
      {wait > 0 ? `Try again in ${wait} s` : children}
    </Button>
  );
}
