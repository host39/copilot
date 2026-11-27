// Saving file bytes the API sent. The browser downloads them through a short-lived `blob:` URL,
// so no URL ever carries the token. Web views ignore such a download, so the Linux and Android
// apps hand the bytes to the shell's `save_file` command (`apps/native/src-tauri/src/files.rs`),
// which asks where to save them with the system's save dialog.

import { LocalError } from "@web/api/errors";
import { isNative } from "@web/config";

/** The header that carries the file's name to `save_file`, percent-encoded. */
export const NAME_HEADER = "x-file-name";

/** Saves through the shell; `false` when the user cancelled the save dialog. */
async function saveNative(blob: Blob, name: string): Promise<boolean> {
  const { invoke } = await import("@tauri-apps/api/core");
  const bytes = new Uint8Array(await blob.arrayBuffer());
  try {
    return await invoke<boolean>("save_file", bytes, {
      headers: { [NAME_HEADER]: encodeURIComponent(name) },
    });
  } catch (error) {
    throw new LocalError(`${name} could not be saved: ${String(error)}`);
  }
}

/**
 * Saves `blob` as a download named `name`. Resolves `false` when the user cancelled the app's
 * save dialog; a failure to save throws a `LocalError` that says why.
 */
export async function saveBlob(blob: Blob, name: string): Promise<boolean> {
  if (isNative()) return saveNative(blob, name);
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.rel = "noopener";
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1_000);
  return true;
}
