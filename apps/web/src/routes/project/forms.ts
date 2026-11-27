// The project forms: create, name and icon, and the side panel's settings. Each mirrors core's
// rules (`Project`), so most mistakes show before sending, and sends only what changed.

import type {
  BuiltinTool,
  ProjectResponse,
  ReasoningLevel,
  UpdateProjectRequest,
} from "@copilot/contracts";
import { z } from "zod";

/** Core's `NAME_MAX_CHARS`. */
export const NAME_MAX_CHARS = 100;
/** Core's `ICON_MAX_CHARS`: one emoji, ZWJ sequences included. */
const ICON_MAX_CHARS = 10;

const name = z
  .string()
  .trim()
  .min(1, "Project name is required.")
  .refine((value) => !/\p{Cc}/u.test(value), "Project name must be a single line.")
  .refine(
    (value) => [...value].length <= NAME_MAX_CHARS,
    `Project name must be at most ${NAME_MAX_CHARS} characters.`
  );

/**
 * Whether `value` is empty or one emoji, by core's test: a short run of characters without
 * whitespace or control characters and with at least one non-ASCII character.
 */
export function isIcon(value: string): boolean {
  const icon = value.trim();
  if (icon === "") return true;
  return (
    [...icon].length <= ICON_MAX_CHARS &&
    !/[\s\p{Cc}]/u.test(icon) &&
    // biome-ignore lint/suspicious/noControlCharactersInRegex: the ASCII range is the point
    /[^\u0000-\u007f]/.test(icon)
  );
}

const icon = z.string().trim().refine(isIcon, "Choose one emoji as the icon.");

/** New project: a name, and optionally an icon and instructions. */
export const projectSchema = z.object({
  name,
  icon,
  instructions: z.string().trim(),
});

/** The rename form: only the fields that changed. */
export function identitySchema(project: ProjectResponse) {
  return z.object({ name, icon }).transform((input) => {
    const body: UpdateProjectRequest = {};
    if (input.name !== project.name) body.name = input.name;
    if (input.icon !== project.icon) body.icon = input.icon;
    return body;
  });
}

/** The instructions form; core trims them. */
export function instructionsBody(project: ProjectResponse, text: string): UpdateProjectRequest {
  const instructions = text.trim();
  return instructions === project.instructions ? {} : { instructions };
}

/** The default model and reasoning; `null` clears either. */
export function modelBody(
  project: ProjectResponse,
  modelId: string | null,
  reasoning: ReasoningLevel | null
): UpdateProjectRequest {
  const body: UpdateProjectRequest = {};
  if (modelId !== project.defaultModelId) body.defaultModelId = modelId;
  if (reasoning !== project.reasoning) body.reasoning = reasoning;
  return body;
}

function same(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value) => right.includes(value));
}

/** The tool servers and built-in tools every chat of the project uses. */
export function toolsBody(
  project: ProjectResponse,
  serverIds: string[],
  builtin: BuiltinTool[]
): UpdateProjectRequest {
  const body: UpdateProjectRequest = {};
  if (!same(serverIds, project.toolServerIds)) body.toolServerIds = serverIds;
  if (!same(builtin, project.builtinTools)) body.builtinTools = builtin;
  return body;
}

/** The page heading: the icon, then the name. */
export function projectLabel(project: Pick<ProjectResponse, "icon" | "name">): string {
  return project.icon ? `${project.icon} ${project.name}` : project.name;
}
