// Saving file bytes the API sent: the browser downloads them through a short-lived `blob:` URL,
// so no URL ever carries the token.

/** Saves `blob` as a download named `name`. */
export function saveBlob(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.rel = "noopener";
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1_000);
}
