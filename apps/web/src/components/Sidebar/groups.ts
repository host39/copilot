// The sidebar's chat lists: recent chats grouped by the day they last changed, and the Scheduled
// entry's unread count.

import type { ChatSummaryResponse } from "@copilot/contracts";

export type DayGroup = {
  /** The local calendar day, `YYYY-MM-DD`. */
  day: string;
  /** "Today", "Yesterday", or the date ("Monday, Nov 23"; with the year when it is another). */
  label: string;
  chats: ChatSummaryResponse[];
};

function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

function label(date: Date, now: Date, locale?: string): string {
  const today = dayKey(now);
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  const key = dayKey(date);
  if (key === today) return "Today";
  if (key === dayKey(yesterday)) return "Yesterday";
  return date.toLocaleDateString(locale, {
    weekday: "long",
    month: "short",
    day: "numeric",
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

/**
 * Chats grouped by the local day of `updatedAt`, newest day first, keeping the server's order
 * within a day. A chat dated after `now` (clocks drift) counts as today.
 */
export function groupByDay(chats: ChatSummaryResponse[], now: Date, locale?: string): DayGroup[] {
  const groups: DayGroup[] = [];
  for (const chat of chats) {
    const parsed = new Date(chat.updatedAt);
    const date = Number.isNaN(parsed.getTime()) || parsed > now ? now : parsed;
    const day = dayKey(date);
    const group = groups.find((item) => item.day === day);
    if (group) {
      group.chats.push(chat);
    } else {
      groups.push({ day, label: label(date, now, locale), chats: [chat] });
    }
  }
  return groups.sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0));
}

/** The Scheduled entry's badge: unread runs, "99+" past 99, nothing at zero. */
export function unreadBadge(chats: Pick<ChatSummaryResponse, "unread">[]): string | null {
  const count = chats.filter((chat) => chat.unread).length;
  if (count === 0) return null;
  return count > 99 ? "99+" : String(count);
}
