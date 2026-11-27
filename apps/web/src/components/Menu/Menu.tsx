import { type KeyboardEvent, type ReactNode, useEffect, useId, useRef, useState } from "react";
import { LuEllipsisVertical } from "react-icons/lu";

export type MenuItem = {
  key: string;
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  /** `danger` for destructive actions (Delete). */
  tone?: "danger";
};

const TONE_CLASS_MAP = { default: "text-fg", danger: "text-danger" } as const;

/**
 * An overflow menu: a button that opens a list of actions below it. Arrow keys move between the
 * actions, Escape or a tap outside closes it, and choosing an action closes it first.
 */
export function Menu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const id = useId();

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  function move(event: KeyboardEvent<HTMLDivElement>) {
    const entries = Array.from(
      root.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []
    );
    const at = entries.indexOf(document.activeElement as HTMLButtonElement);
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
      button.current?.focus();
    } else if (step !== undefined && entries.length > 0) {
      event.preventDefault();
      entries[(at + step + entries.length) % entries.length]?.focus();
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      entries[event.key === "Home" ? 0 : entries.length - 1]?.focus();
    }
  }

  return (
    <div ref={root} className="relative">
      <button
        ref={button}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((value) => !value)}
        className="icon-button"
      >
        <LuEllipsisVertical aria-hidden />
      </button>
      {open ? (
        <div
          id={id}
          role="menu"
          aria-label={label}
          tabIndex={-1}
          onKeyDown={move}
          className="absolute top-full right-0 z-20 mt-1 min-w-56 rounded-xl border border-line bg-canvas p-1 shadow-lg"
        >
          {items.map((item) => (
            <button
              key={item.key}
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className={`flex min-h-10 w-full items-center gap-3 rounded-lg px-3 text-left text-sm hover:bg-raised focus-visible:bg-raised focus-visible:outline-none ${TONE_CLASS_MAP[item.tone ?? "default"]}`}
            >
              {item.icon ? (
                <span aria-hidden className="text-muted">
                  {item.icon}
                </span>
              ) : null}
              {item.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
