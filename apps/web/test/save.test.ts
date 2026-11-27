// Saving downloads in the Linux and Android apps: the bytes go to the shell's `save_file` command
// (the system's save dialog), because web views ignore `<a download>` on a `blob:` URL.

import { describe, expect, rs, test } from "@rstest/core";
import { describeError } from "@web/api/errors";
import { NAME_HEADER, saveBlob } from "@web/components/Files/save";

const { invoke } = rs.hoisted(() => ({
  invoke: rs.fn(
    async (_command: string, _args?: unknown, _options?: { headers?: Record<string, string> }) =>
      true as unknown
  ),
}));
rs.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke }));

describe("saving a download in the apps", () => {
  test("hands the bytes and the encoded name to the shell's save command", async () => {
    invoke.mockClear();
    const saved = await saveBlob(new Blob(["hello"], { type: "text/plain" }), "Trip plans ✈/x.md");
    expect(saved).toBe(true);
    const [command, bytes, options] = invoke.mock.calls[0] ?? [];
    expect(command).toBe("save_file");
    expect(bytes).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(bytes as Uint8Array)).toBe("hello");
    expect(options?.headers?.[NAME_HEADER]).toBe(encodeURIComponent("Trip plans ✈/x.md"));
  });

  test("a cancelled dialog is no error", async () => {
    invoke.mockResolvedValueOnce(false);
    expect(await saveBlob(new Blob(["x"]), "notes.txt")).toBe(false);
  });

  test("a failure says which file could not be saved, and why", async () => {
    invoke.mockRejectedValueOnce("Permission denied (os error 13)");
    const failure = await saveBlob(new Blob(["x"]), "notes.txt").catch((error: unknown) => error);
    expect(describeError(failure).message).toBe(
      "notes.txt could not be saved: Permission denied (os error 13)"
    );
  });
});
