import { useMutation } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchFileContent } from "@web/api/routes/files";
import { saveBlob } from "@web/components/Files/save";
import { LuPaperclip } from "react-icons/lu";

/** A file in a message: its name, which downloads it (bytes come with the token). */
export function FileChip({ fileId, name }: { fileId: string; name: string }) {
  const download = useMutation({
    mutationFn: async () => saveBlob(await fetchFileContent(fileId), name),
  });
  return (
    <span className="inline-flex flex-col">
      <button
        type="button"
        onClick={() => download.mutate()}
        disabled={download.isPending}
        title={`Download ${name}`}
        className="inline-flex min-h-9 max-w-64 items-center gap-2 rounded-lg border border-line bg-canvas px-3 text-sm hover:bg-raised disabled:opacity-60"
      >
        <LuPaperclip aria-hidden className="shrink-0 text-muted" />
        <span className="truncate">{name}</span>
      </button>
      {download.isError ? (
        <span role="alert" className="field-error">
          {describeError(download.error).message}
        </span>
      ) : null}
    </span>
  );
}
