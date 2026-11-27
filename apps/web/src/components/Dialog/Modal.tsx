import { type ReactNode, useEffect, useId, useRef } from "react";
import { LuX } from "react-icons/lu";

type ModalProps = {
  open: boolean;
  title: string;
  /** Called on the Close button, Escape, and a backdrop tap. */
  onClose: () => void;
  children: ReactNode;
  /** `side` slides in at the right edge, full height (a drawer); `center` is a centered sheet. */
  placement?: "center" | "side";
  /** Keeps the dialog open on a backdrop tap while an action runs. */
  busy?: boolean;
};

const PLACEMENT_CLASS_MAP = {
  center:
    "m-auto max-h-[min(40rem,calc(100dvh-2rem))] w-[min(32rem,calc(100vw-2rem))] rounded-2xl border",
  side: "my-0 mr-0 ml-auto h-dvh max-h-none w-[min(28rem,100vw)] max-w-none border-l",
} as const;

/**
 * A modal `<dialog>` with a title and a Close button: the browser traps focus, makes the page
 * inert, and closes it on Escape. Its content mounts only while it is open, so forms start fresh.
 */
export function Modal({
  open,
  title,
  onClose,
  children,
  placement = "center",
  busy = false,
}: ModalProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: a backdrop tap is the pointer shortcut; keyboards use Escape or Close
    <dialog
      ref={dialog}
      aria-labelledby={titleId}
      onClose={() => {
        if (open) onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
      className={`border-line bg-canvas p-0 text-fg shadow-xl backdrop:bg-black/50 ${PLACEMENT_CLASS_MAP[placement]}`}
    >
      {open ? (
        <div className="flex h-full max-h-[inherit] min-h-0 flex-col">
          <div className="flex shrink-0 items-center justify-between gap-3 border-b border-line px-5 py-3">
            <h2 id={titleId} className="text-lg font-semibold tracking-tight">
              {title}
            </h2>
            <button
              type="button"
              onClick={onClose}
              disabled={busy}
              aria-label="Close"
              className="flex size-10 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-fg disabled:opacity-60"
            >
              <LuX aria-hidden />
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-5">{children}</div>
        </div>
      ) : null}
    </dialog>
  );
}
