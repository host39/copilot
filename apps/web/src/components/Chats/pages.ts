// Chat lists (`GET /chats`) a page at a time, for the sidebar, a project's chats, and the runs of
// a schedule.

import type { ChatSummaryResponse, PageResponse } from "@copilot/contracts";
import { useInfiniteQuery } from "@tanstack/react-query";
import { fetchChats } from "@web/api/routes/chats";
import { type ChatFilter, keys } from "@web/api/routes/keys";

/** The pages of the chats `filter` selects, most recently updated first. */
export function useChatPages(filter: ChatFilter, enabled = true) {
  const query = useInfiniteQuery({
    queryKey: keys.chatList(filter),
    queryFn: ({ pageParam, signal }) =>
      fetchChats({ ...filter, cursor: pageParam ?? undefined }, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (page: PageResponse<ChatSummaryResponse>) => page.next,
    enabled,
  });
  return { ...query, chats: flatten(query.data?.pages) };
}

/** The chats of every loaded page, without repeats (a chat that moved up between two pages). */
export function flatten(
  pages: PageResponse<ChatSummaryResponse>[] | undefined
): ChatSummaryResponse[] {
  const seen = new Set<string>();
  const chats: ChatSummaryResponse[] = [];
  for (const chat of (pages ?? []).flatMap((page) => page.items)) {
    if (seen.has(chat.id)) continue;
    seen.add(chat.id);
    chats.push(chat);
  }
  return chats;
}
