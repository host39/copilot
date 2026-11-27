// The rules a modal `<dialog>` follows while an action in it runs (saving, deleting): Escape,
// a backdrop tap, and Close leave it open, so the user sees the action end and any error it
// returns. A sheet whose form holds unsaved changes asks before it discards them, and only a
// real tap on the backdrop (not a drag out of a field) counts as one.

import { useRef } from "react";

/** The `cancel` handler of a modal `<dialog>` (Escape): while `busy`, the dialog stays open. */
export function holdWhileBusy(busy: boolean): (event: { preventDefault: () => void }) => void {
  return (event) => {
    if (busy) event.preventDefault();
  };
}

/**
 * What a modal `<dialog>`'s `close` event means: nothing when the page closed it itself
 * (`open` is already false), `reopen` when the browser closed it while an action runs (a browser
 * may skip a prevented Escape when it is pressed again), else `close` (tell the page).
 */
export function afterClose(open: boolean, busy: boolean): "ignore" | "reopen" | "close" {
  if (!open) return "ignore";
  return busy ? "reopen" : "close";
}

/**
 * Whether a click on a modal `<dialog>` is a tap on its backdrop: the press that began it and the
 * click both landed on the dialog element itself (the backdrop counts as the dialog). A drag that
 * starts in a field and ends on the backdrop also clicks the dialog, their nearest common
 * ancestor, so the press is checked too: selecting text past the edge never closes the sheet.
 */
export function isBackdropTap(
  pressed: EventTarget | null,
  clicked: EventTarget | null,
  dialog: EventTarget | null
): boolean {
  return dialog !== null && pressed === dialog && clicked === dialog;
}

/**
 * The `onPointerDown` and `onClick` handlers of a modal `<dialog>` that call `onTap` on a backdrop
 * tap only ([`isBackdropTap`]).
 */
export function useBackdropTap(onTap: () => void) {
  const pressed = useRef<EventTarget | null>(null);
  return {
    onPointerDown: (event: { target: EventTarget | null }) => {
      pressed.current = event.target;
    },
    onClick: (event: { target: EventTarget | null; currentTarget: EventTarget | null }) => {
      const tap = isBackdropTap(pressed.current, event.target, event.currentTarget);
      pressed.current = null;
      if (tap) onTap();
    },
  };
}

/**
 * What a request to close a modal sheet does (Escape, a backdrop tap, Close): nothing while an
 * action runs, a "discard your changes?" question while its form holds unsaved changes, else
 * close.
 */
export function closeRequest(busy: boolean, dirty: boolean): "ignore" | "ask" | "close" {
  if (busy) return "ignore";
  return dirty ? "ask" : "close";
}
