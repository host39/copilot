import type { ChatSummaryResponse } from "@copilot/contracts";
import { link } from "@web/routes/path";
import type { ReactNode } from "react";
import { NavLink } from "react-router-dom";

/** A chat's title, or a stand-in while the first message has not named it. */
export function chatTitle(chat: Pick<ChatSummaryResponse, "title">): string {
  return chat.title.trim() || "New chat";
}

/**
 * The dots of a chat row: "needs approval" while a tool call waits for a decision, else "unread"
 * when it changed since the user last opened it. Each has a name for screen readers.
 */
export function ChatDots({
  chat,
}: {
  chat: Pick<ChatSummaryResponse, "unread" | "awaitingApproval">;
}) {
  if (chat.awaitingApproval) {
    return (
      <span className="flex shrink-0 items-center" title="Needs approval">
        <span aria-hidden className="size-2 rounded-full bg-warn" />
        <span className="sr-only">Needs approval</span>
      </span>
    );
  }
  if (chat.unread) {
    return (
      <span className="flex shrink-0 items-center" title="Unread">
        <span aria-hidden className="size-2 rounded-full bg-accent" />
        <span className="sr-only">Unread</span>
      </span>
    );
  }
  return null;
}

type ChatRowProps = {
  chat: ChatSummaryResponse;
  /** Shown after the title (the project, the time). */
  detail?: ReactNode;
  /** `nav` is the sidebar's compact row; `list` a row of a page's chat list. */
  variant?: "nav" | "list";
};

/** One chat in a list: its title, the unread and needs-approval dots, and a link to it. */
export function ChatRow({ chat, detail, variant = "nav" }: ChatRowProps) {
  const unread = chat.unread || chat.awaitingApproval;
  return (
    <NavLink to={link.chat(chat.id)} className={variant === "nav" ? "nav-item" : "list-row"}>
      <span className={`min-w-0 flex-1 truncate ${unread ? "font-semibold" : ""}`}>
        {chatTitle(chat)}
      </span>
      {detail ? <span className="shrink-0 text-xs text-muted">{detail}</span> : null}
      <ChatDots chat={chat} />
    </NavLink>
  );
}
