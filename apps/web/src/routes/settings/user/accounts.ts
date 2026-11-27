// User administration in Settings: account states, role labels, temporary passwords, and the
// profile form, which sends only what changed.

import {
  type UpdateUserRequest,
  type UserResponse,
  type UserRole,
  updateUserRequestSchema,
} from "@copilot/contracts";
import type { BadgeTone } from "@web/components/Badge/Badge";

/** Every role with what it means in Copilot v1 (`manager` acts as a plain user). */
export const ROLES: readonly { value: UserRole; label: string; hint: string }[] = [
  { value: "user", label: "User", hint: "Chats, projects, providers, and tools of their own." },
  { value: "manager", label: "Manager", hint: "The same as a user for now." },
  { value: "administrator", label: "Administrator", hint: "Also manages every account." },
];

export function roleLabel(role: UserRole): string {
  return ROLES.find((item) => item.value === role)?.label ?? role;
}

/** Whether failed sign-ins locked the account at `now`. */
export function isLocked(user: UserResponse, now: number = Date.now()): boolean {
  return user.lockedUntil !== undefined && Date.parse(user.lockedUntil) > now;
}

/** The badges an account shows in the list and on its page. */
export function accountBadges(
  user: UserResponse,
  now: number = Date.now()
): { label: string; tone: BadgeTone }[] {
  const badges: { label: string; tone: BadgeTone }[] = [];
  if (user.role === "administrator") badges.push({ label: "Administrator", tone: "accent" });
  else if (user.role === "manager") badges.push({ label: "Manager", tone: "neutral" });
  if (user.disabled) badges.push({ label: "Disabled", tone: "danger" });
  if (isLocked(user, now)) badges.push({ label: "Locked", tone: "warn" });
  return badges;
}

/** Letters and digits that cannot be mistaken for each other when read aloud or copied by hand. */
const ALPHABET = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";

/**
 * A random temporary password: four groups of four characters ("k7Qm-2xPa-..."), 19 bytes, from
 * the browser's cryptographic random source. Rejection sampling keeps every character equally
 * likely.
 */
export function temporaryPassword(random: (count: number) => Uint8Array = randomBytes): string {
  const limit = 256 - (256 % ALPHABET.length);
  const chars: string[] = [];
  while (chars.length < 16) {
    for (const byte of random(32)) {
      if (byte < limit && chars.length < 16) chars.push(ALPHABET[byte % ALPHABET.length] as string);
    }
  }
  return [0, 4, 8, 12].map((start) => chars.slice(start, start + 4).join("")).join("-");
}

function randomBytes(count: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(count));
}

/** The profile form of `user` (display name and role); parses to a `PATCH` body of what changed. */
export function profileSchema(user: UserResponse) {
  return updateUserRequestSchema
    .pick({ displayName: true, role: true })
    .required()
    .transform((input): UpdateUserRequest => {
      const body: UpdateUserRequest = {};
      if (input.displayName !== user.displayName) body.displayName = input.displayName;
      if (input.role !== user.role) body.role = input.role;
      return body;
    });
}

/** The reset form: a new temporary password with rustauth's rules. */
export const resetSchema = updateUserRequestSchema.pick({ password: true }).required();
