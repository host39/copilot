// How other screens open the new chat screen with a project in its draft.

/** The search parameter that puts the new chat into a project (the sidebar's `+` per project). */
export const PROJECT_PARAM = "project";

/** The new chat screen for `projectId` (`/?project=<id>`). */
export function newChatLink(projectId: string): string {
  return `/?${new URLSearchParams({ [PROJECT_PARAM]: projectId })}`;
}
