// Modal dialogs while an action in them runs: Escape, a backdrop tap, and Close leave them open.
// With unsaved changes they ask first, and a drag out of a field is not a backdrop tap.

import { describe, expect, test } from "@rstest/core";
import {
  afterClose,
  closeRequest,
  holdWhileBusy,
  isBackdropTap,
} from "@web/components/Dialog/dialog";

function cancel() {
  let prevented = false;
  return {
    event: {
      preventDefault: () => {
        prevented = true;
      },
    },
    prevented: () => prevented,
  };
}

describe("a dialog while its action runs", () => {
  test("Escape does not close it while busy, and does otherwise", () => {
    const busy = cancel();
    holdWhileBusy(true)(busy.event);
    expect(busy.prevented()).toBe(true);
    const idle = cancel();
    holdWhileBusy(false)(idle.event);
    expect(idle.prevented()).toBe(false);
  });

  test("a close the browser forced while busy reopens it; the page's own close is ignored", () => {
    expect(afterClose(true, true)).toBe("reopen");
    expect(afterClose(true, false)).toBe("close");
    expect(afterClose(false, true)).toBe("ignore");
    expect(afterClose(false, false)).toBe("ignore");
  });
});

describe("a backdrop tap", () => {
  const dialog = { name: "dialog" } as unknown as EventTarget;
  const field = { name: "textarea" } as unknown as EventTarget;

  test("is a press and a release both on the dialog itself", () => {
    expect(isBackdropTap(dialog, dialog, dialog)).toBe(true);
  });

  test("is not a drag that starts in a field and ends on the backdrop", () => {
    // The click goes to the dialog, the common ancestor of the press and the release.
    expect(isBackdropTap(field, dialog, dialog)).toBe(false);
  });

  test("is not a click inside the sheet, nor one without a press (a keyboard click)", () => {
    expect(isBackdropTap(dialog, field, dialog)).toBe(false);
    expect(isBackdropTap(null, dialog, dialog)).toBe(false);
    expect(isBackdropTap(null, null, null)).toBe(false);
  });
});

describe("closing a sheet", () => {
  test("does nothing while its action runs, even with unsaved changes", () => {
    expect(closeRequest(true, false)).toBe("ignore");
    expect(closeRequest(true, true)).toBe("ignore");
  });

  test("asks before discarding unsaved changes, and closes at once otherwise", () => {
    expect(closeRequest(false, true)).toBe("ask");
    expect(closeRequest(false, false)).toBe("close");
  });
});
