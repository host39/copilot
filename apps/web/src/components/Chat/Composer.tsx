import Button from "@web/components/Button/Button";
import { useState } from "react";
import { LuSendHorizontal } from "react-icons/lu";

type ComposerProps = {
  /** Called with the trimmed text; the field clears when it resolves to `true`. */
  onSend: (text: string) => Promise<boolean>;
  busy?: boolean;
  disabled?: boolean;
  placeholder?: string;
};

/** The message field. Enter sends; Shift+Enter starts a new line. */
export function Composer({ onSend, busy = false, disabled = false, placeholder }: ComposerProps) {
  const [text, setText] = useState("");
  const blocked = busy || disabled;

  async function submit() {
    const trimmed = text.trim();
    if (trimmed === "" || blocked) return;
    if (await onSend(trimmed)) setText("");
  }

  return (
    <form
      className="flex items-end gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label className="sr-only" htmlFor="composer">
        Message
      </label>
      <textarea
        id="composer"
        name="text"
        rows={2}
        value={text}
        placeholder={placeholder ?? "Message"}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void submit();
          }
        }}
        className="field-input mt-0 max-h-48 min-h-12 resize-y"
      />
      <Button type="submit" busy={busy} disabled={disabled || text.trim() === ""} aria-label="Send">
        <LuSendHorizontal aria-hidden />
      </Button>
    </form>
  );
}
