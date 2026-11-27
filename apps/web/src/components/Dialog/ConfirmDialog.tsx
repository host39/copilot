import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { type ReactNode, useEffect, useId, useRef } from "react";

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  children: ReactNode;
  /** The confirm button's label ("Delete provider"). */
  confirm: string;
  /** `danger` for destructive actions. */
  tone?: "danger" | "primary";
  busy?: boolean;
  /** Why the last attempt failed. */
  error?: string;
  onConfirm: () => void;
  /** Called on Cancel, Escape, and a backdrop tap. */
  onCancel: () => void;
};

/**
 * A modal confirmation (`<dialog>`): the browser traps focus, makes the page inert, and closes it
 * on Escape. Cancel comes first, so `showModal` focuses it and Enter never confirms a destructive
 * action by accident.
 */
export function ConfirmDialog({
  open,
  title,
  children,
  confirm,
  tone = "danger",
  busy = false,
  error,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const bodyId = useId();

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: a backdrop tap is the pointer shortcut; keyboards use Escape or Cancel
    <dialog
      ref={dialog}
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      onClose={() => {
        if (open) onCancel();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
      className="m-auto w-[min(28rem,calc(100vw-2rem))] rounded-2xl border border-line bg-canvas p-0 text-fg shadow-xl backdrop:bg-black/50"
    >
      <div className="space-y-4 p-6">
        <h2 id={titleId} className="text-lg font-semibold tracking-tight">
          {title}
        </h2>
        <div id={bodyId} className="space-y-2 text-sm text-muted">
          {children}
        </div>
        <FormAlert>{error}</FormAlert>
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <Button variant="secondary" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button variant={tone} busy={busy} onClick={onConfirm}>
            {confirm}
          </Button>
        </div>
      </div>
    </dialog>
  );
}
