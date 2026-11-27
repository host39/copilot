// Modal dialogs while an action in them runs: Escape, a backdrop tap, and Close leave them open.

import { describe, expect, test } from "@rstest/core";
import { afterClose, holdWhileBusy } from "@web/components/Dialog/dialog";

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
