import type { Capabilities } from "@copilot/contracts";
import { describeError } from "@web/api/errors";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import {
  type Attachment,
  attachments,
  canSend,
  enterSends,
  finePointer,
  mediaType,
  unreadable,
} from "@web/components/Chat/composer";
import { size } from "@web/routes/project/files";
import { type ReactNode, useEffect, useId, useReducer, useRef, useState } from "react";
import { LuPaperclip, LuSendHorizontal, LuSquare, LuX } from "react-icons/lu";

/** An attached file with its bytes, as the composer hands it to `onSend`. */
export type Picked = Attachment & { file: File };

/** Stores a picked file at once (a chat's attachments); `onProgress` hears the bytes sent. */
export type Upload = (
  file: File,
  name: string,
  onProgress: (loaded: number) => void
) => Promise<{ id: string; mediaType: string }>;

type ComposerProps = {
  /** Called with the trimmed text and the ready files; the field clears when it resolves `true`. */
  onSend: (text: string, files: Picked[]) => Promise<boolean>;
  /** Stops the answer being written; the Send button becomes Stop while `running`. */
  onStop?: () => void;
  running?: boolean;
  /** Sending: the button spins. */
  busy?: boolean;
  disabled?: boolean;
  placeholder?: string;
  /** The model's capabilities, for the warning chips on files it cannot read. */
  capabilities?: Capabilities | null;
  /** Uploads each file as it is picked; without it, files wait for `onSend` (a new chat). */
  upload?: Upload;
  /** Deletes an uploaded file the user took off the message. */
  discard?: (fileId: string) => void;
  /** The tools button, before attach. */
  tools?: ReactNode;
};

/** Whether the browser sizes the text area to its text itself (`field-sizing: content`). */
function sizesItself(): boolean {
  return typeof CSS !== "undefined" && CSS.supports("field-sizing", "content");
}

/** A file on the message: name, upload progress or failure, and a warning when the model cannot read it. */
function Chip({
  item,
  warning,
  onRemove,
}: {
  item: Attachment;
  warning: string | null;
  onRemove: () => void;
}) {
  const percent = item.size > 0 ? Math.round((item.loaded / item.size) * 100) : 0;
  return (
    <li className="inline-flex max-w-full items-center gap-2 rounded-lg border border-line bg-canvas py-1 pr-1 pl-2 text-xs">
      <LuPaperclip aria-hidden className="shrink-0 text-muted" />
      <span className="max-w-40 truncate" title={item.name}>
        {item.name}
      </span>
      <span className="text-muted">
        {item.status === "uploading" ? `${percent}%` : size(item.size)}
      </span>
      {item.status === "failed" ? <span className="text-danger">{item.error}</span> : null}
      {warning ? <Badge tone="warn">{warning}</Badge> : null}
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Remove ${item.name}`}
        title="Remove"
        className="flex size-7 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-fg"
      >
        <LuX aria-hidden />
      </button>
    </li>
  );
}

/**
 * The message field: an auto-growing text area, attach (always shown; a file the model cannot
 * read gets a warning chip), and Send, which becomes Stop while an answer is written. With a
 * mouse or trackpad Enter sends and Shift+Enter adds a line; on touch screens Enter adds a line.
 */
export function Composer({
  onSend,
  onStop,
  running = false,
  busy = false,
  disabled = false,
  placeholder,
  capabilities,
  upload,
  discard,
  tools,
}: ComposerProps) {
  const [text, setText] = useState("");
  const [list, dispatch] = useReducer(attachments, []);
  const files = useRef(new Map<string, File>());
  const counter = useRef(0);
  const field = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const id = useId();
  const ready = canSend(text, list);
  const blocked = busy || disabled || running;

  // Browsers without `field-sizing` grow the field from its scroll height (capped by max-height).
  // biome-ignore lint/correctness/useExhaustiveDependencies: the height follows the text
  useEffect(() => {
    const element = field.current;
    if (!element || sizesItself()) return;
    element.style.height = "auto";
    element.style.height = `${element.scrollHeight}px`;
  }, [text]);

  function pick(picked: FileList | File[] | null) {
    for (const file of Array.from(picked ?? [])) {
      counter.current += 1;
      const key = `file-${counter.current}`;
      files.current.set(key, file);
      dispatch({
        type: "add",
        item: {
          key,
          name: file.name,
          mediaType: mediaType(file.name, file.type),
          size: file.size,
          status: upload ? "uploading" : "ready",
          loaded: 0,
        },
      });
      if (!upload) continue;
      upload(file, file.name, (loaded) => dispatch({ type: "progress", key, loaded }))
        .then((saved) =>
          dispatch({ type: "ready", key, fileId: saved.id, mediaType: saved.mediaType })
        )
        .catch((error: unknown) =>
          dispatch({ type: "failed", key, error: describeError(error).message })
        );
    }
  }

  function remove(item: Attachment) {
    files.current.delete(item.key);
    dispatch({ type: "remove", key: item.key });
    if (item.fileId) discard?.(item.fileId);
  }

  async function submit() {
    if (!ready || blocked) return;
    const sent = list.flatMap((item) => {
      const file = files.current.get(item.key);
      return item.status === "ready" && file ? [{ ...item, file }] : [];
    });
    if (await onSend(text.trim(), sent)) {
      setText("");
      files.current.clear();
      dispatch({ type: "clear" });
    }
  }

  return (
    <form
      className="space-y-2"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
      onDragOver={(event) => {
        if (!disabled && event.dataTransfer.types.includes("Files")) event.preventDefault();
      }}
      onDrop={(event) => {
        if (disabled || event.dataTransfer.files.length === 0) return;
        event.preventDefault();
        pick(event.dataTransfer.files);
      }}
    >
      {list.length > 0 ? (
        <ul aria-label="Attached files" className="flex flex-wrap gap-2">
          {list.map((item) => (
            <Chip
              key={item.key}
              item={item}
              warning={unreadable(item.mediaType, capabilities)}
              onRemove={() => remove(item)}
            />
          ))}
        </ul>
      ) : null}
      <div className="flex items-end gap-2">
        <input
          ref={picker}
          type="file"
          multiple
          className="sr-only"
          tabIndex={-1}
          aria-hidden
          onChange={(event) => {
            pick(event.target.files);
            event.target.value = "";
          }}
        />
        {tools}
        <button
          type="button"
          onClick={() => picker.current?.click()}
          disabled={disabled}
          aria-label="Attach files"
          title="Attach files"
          className="icon-button"
        >
          <LuPaperclip aria-hidden />
        </button>
        <label className="sr-only" htmlFor={id}>
          Message
        </label>
        <textarea
          ref={field}
          id={id}
          name="text"
          rows={1}
          value={text}
          placeholder={placeholder ?? "Message"}
          disabled={disabled}
          onChange={(event) => setText(event.target.value)}
          onPaste={(event) => {
            const pasted = event.clipboardData.files;
            if (pasted.length > 0 && !disabled) {
              event.preventDefault();
              pick(pasted);
            }
          }}
          onKeyDown={(event) => {
            const fine = finePointer();
            if (event.key !== "Enter" || event.shiftKey || !fine) return;
            // Enter never adds a line where it sends, even while an answer is written.
            event.preventDefault();
            if (enterSends(event.nativeEvent, fine)) void submit();
          }}
          className="field-input mt-0 max-h-60 min-h-11 resize-none [field-sizing:content]"
        />
        {running ? (
          <Button onClick={onStop} aria-label="Stop" title="Stop">
            <LuSquare aria-hidden />
          </Button>
        ) : (
          <Button
            type="submit"
            busy={busy}
            disabled={disabled || !ready}
            aria-label="Send"
            title="Send"
          >
            <LuSendHorizontal aria-hidden />
          </Button>
        )}
      </div>
    </form>
  );
}
