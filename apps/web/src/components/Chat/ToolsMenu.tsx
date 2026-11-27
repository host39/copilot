import type { BuiltinTool, ToolOverride, ToolServerResponse } from "@copilot/contracts";
import { builtinOn, serverRows, setBuiltin, setServer } from "@web/components/Chat/tools";
import path from "@web/routes/path";
import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { LuPlug, LuPlus, LuTriangleAlert } from "react-icons/lu";
import { Link } from "react-router-dom";

/** What a chat (or the new chat draft) chooses on top of its project. */
export type ToolChoice = { toolOverrides: ToolOverride[]; builtinTools: BuiltinTool[] | null };

type ToolsMenuProps = {
  servers: ToolServerResponse[] | undefined;
  project: { toolServerIds: string[]; builtinTools: BuiltinTool[] } | null | undefined;
  choice: ToolChoice;
  /** Stores a change; the menu shows it at once. */
  onChange: (choice: ToolChoice) => void;
  /** Tools are on but the model cannot call them. */
  warning: boolean;
  disabled?: boolean;
};

/**
 * The composer's tools button (a `+` with a plug) and its menu: the tool servers and the built-in
 * Schedules tools with checkboxes, the project's servers checked and labeled "from project"
 * (unchecking one stores a chat override), and "Add tool server..." last. The button shows a
 * warning badge while the model cannot call the tools that are on.
 */
export function ToolsMenu({
  servers,
  project,
  choice,
  onChange,
  warning,
  disabled = false,
}: ToolsMenuProps) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const id = useId();
  const rows = serverRows(servers, project, choice.toolOverrides);
  const builtin = builtinOn(choice.builtinTools, project);

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLInputElement>("input")?.focus();
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  function keys(event: KeyboardEvent<HTMLFieldSetElement>) {
    if (event.key !== "Escape") return;
    // The menu takes the key, so Escape never also stops the answer.
    event.preventDefault();
    setOpen(false);
    button.current?.focus();
  }

  const label = warning ? "Tools (the model cannot call them)" : "Tools";
  return (
    <div ref={root} className="relative">
      <button
        ref={button}
        type="button"
        aria-label={label}
        title={label}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
        className="icon-button relative"
      >
        <LuPlus aria-hidden className="size-3" />
        <LuPlug aria-hidden />
        {warning ? (
          <LuTriangleAlert
            aria-hidden
            className="absolute -top-0.5 -right-0.5 size-3.5 text-warn"
          />
        ) : null}
      </button>
      {open ? (
        <fieldset
          id={id}
          onKeyDown={keys}
          className="absolute bottom-full left-0 z-20 mb-1 w-72 max-w-[calc(100vw-2rem)] rounded-xl border border-line bg-canvas p-1 shadow-lg"
        >
          <legend className="sr-only">Tools</legend>
          {rows.length === 0 ? (
            <p className="px-3 py-2 text-sm text-muted">No tool servers yet.</p>
          ) : null}
          {rows.map((row) => (
            <label
              key={row.id}
              className="flex min-h-10 cursor-pointer items-center gap-3 rounded-lg px-3 text-sm hover:bg-raised"
            >
              <input
                type="checkbox"
                checked={row.on}
                onChange={(event) =>
                  onChange({
                    ...choice,
                    toolOverrides: setServer(
                      project,
                      choice.toolOverrides,
                      row.id,
                      event.target.checked
                    ),
                  })
                }
              />
              <span className="min-w-0 flex-1 truncate">{row.name}</span>
              {row.review ? <span className="text-xs text-warn">Review</span> : null}
              {row.fromProject ? <span className="text-xs text-muted">from project</span> : null}
            </label>
          ))}
          <label className="flex min-h-10 cursor-pointer items-center gap-3 rounded-lg px-3 text-sm hover:bg-raised">
            <input
              type="checkbox"
              checked={builtin.includes("schedules")}
              onChange={(event) =>
                onChange({
                  ...choice,
                  builtinTools: setBuiltin(builtin, "schedules", event.target.checked),
                })
              }
            />
            <span className="min-w-0 flex-1">Schedules</span>
            <span className="text-xs text-muted">Built-in</span>
          </label>
          <div className="my-1 h-px bg-line" />
          <Link
            to={path.toolServerNew}
            className="flex min-h-10 items-center gap-3 rounded-lg px-3 text-sm hover:bg-raised"
          >
            <LuPlus aria-hidden className="text-muted" />
            Add tool server...
          </Link>
        </fieldset>
      ) : null}
    </div>
  );
}
