// How other screens open the new schedule form with a target or a chat to prefill it.

import path from "@web/routes/path";

/** The search parameter that makes the new schedule run in a project (the project page). */
export const PROJECT_PARAM = "project";
/** The search parameter that prefills the form from a chat ("Schedule this prompt..."). */
export const CHAT_PARAM = "chat";

/** The new schedule form, in a project or prefilled from a chat. */
export function newScheduleLink(from: { projectId?: string; chatId?: string } = {}): string {
  const params = new URLSearchParams();
  if (from.projectId) params.set(PROJECT_PARAM, from.projectId);
  if (from.chatId) params.set(CHAT_PARAM, from.chatId);
  const query = params.toString();
  return query ? `${path.scheduleNew}?${query}` : path.scheduleNew;
}
