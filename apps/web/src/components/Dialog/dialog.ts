// The rules a modal `<dialog>` follows while an action in it runs (saving, deleting): Escape,
// a backdrop tap, and Close leave it open, so the user sees the action end and any error it
// returns.

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
