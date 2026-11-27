import type { UserResponse, UserRole } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { deleteUser, fetchUser, unlockUser, updateUser } from "@web/api/routes/users";
import { useUser } from "@web/auth/session";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { TextField } from "@web/components/Form/TextField";
import { QueryState } from "@web/components/Settings/QueryState";
import { RolePicker } from "@web/components/Settings/RolePicker";
import { Section } from "@web/components/Settings/Section";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { TemporaryPassword } from "@web/components/Settings/TemporaryPassword";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { dateTime } from "@web/routes/settings/format";
import {
  accountBadges,
  isLocked,
  profileSchema,
  resetSchema,
} from "@web/routes/settings/user/accounts";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

const BACK = { to: path.users, label: "Users" };

/** Shows a saved account at once and marks the list stale. */
function useStore() {
  const client = useQueryClient();
  return (saved: UserResponse) => {
    client.setQueryData(keys.user(saved.id), saved);
    void client.invalidateQueries({ queryKey: keys.users });
  };
}

function Profile({ user }: { user: UserResponse }) {
  const store = useStore();
  const [role, setRole] = useState<UserRole>(user.role);
  const [notice, setNotice] = useState<string>();
  const form = useSubmit(
    profileSchema(user),
    async (body) => (Object.keys(body).length === 0 ? user : updateUser(user.id, body)),
    (saved) => {
      store(saved);
      setNotice("Saved.");
    }
  );

  return (
    <Section title="Profile" description="Saving a change signs this account out on every device.">
      <form
        key={user.updatedAt}
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          void form.submit({
            displayName: text(new FormData(event.currentTarget), "displayName"),
            role,
          });
        }}
      >
        <FormAlert>{form.alert ?? form.errors.role}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <TextField
          label="Name"
          name="displayName"
          autoComplete="off"
          defaultValue={user.displayName}
          error={form.errors.displayName}
        />
        <RolePicker value={role} onChange={setRole} />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            Save profile
          </Button>
        </div>
      </form>
    </Section>
  );
}

function Access({ user }: { user: UserResponse }) {
  const store = useStore();
  const client = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [notice, setNotice] = useState<string>();
  const disable = useMutation({
    mutationFn: (disabled: boolean) => updateUser(user.id, { disabled }),
    onSuccess: (saved) => {
      store(saved);
      setConfirming(false);
      setNotice(saved.disabled ? "The account is disabled." : "The account is enabled again.");
    },
  });
  const unlock = useMutation({
    mutationFn: () => unlockUser(user.id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: keys.users });
      setNotice("The account is unlocked.");
    },
  });
  const locked = isLocked(user);
  const failure = unlock.error ?? (confirming ? null : disable.error);

  return (
    <Section
      title="Access"
      description={
        user.disabled
          ? "This account cannot sign in."
          : locked && user.lockedUntil
            ? `Too many failed sign-ins: locked until ${dateTime(user.lockedUntil)}.`
            : "This account can sign in."
      }
    >
      <div className="space-y-4">
        <FormAlert>{failure ? describeError(failure).message : null}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <div className="flex flex-wrap gap-2">
          {user.disabled ? (
            <Button
              variant="secondary"
              busy={disable.isPending}
              onClick={() => {
                setNotice(undefined);
                disable.mutate(false);
              }}
            >
              Enable account
            </Button>
          ) : (
            <Button
              variant="secondary"
              onClick={() => {
                setNotice(undefined);
                setConfirming(true);
              }}
            >
              Disable account
            </Button>
          )}
          {locked ? (
            <Button
              variant="secondary"
              busy={unlock.isPending}
              onClick={() => {
                setNotice(undefined);
                unlock.mutate();
              }}
            >
              Unlock
            </Button>
          ) : null}
        </div>
      </div>
      <ConfirmDialog
        open={confirming}
        title={`Disable @${user.username}?`}
        confirm="Disable account"
        busy={disable.isPending}
        error={disable.error ? describeError(disable.error).message : undefined}
        onConfirm={() => disable.mutate(true)}
        onCancel={() => {
          setConfirming(false);
          disable.reset();
        }}
      >
        <p>
          They are signed out on every device and cannot sign in until you enable the account again.
          Their chats, projects, and files stay; their schedules stop running.
        </p>
      </ConfirmDialog>
    </Section>
  );
}

function Reset({ user }: { user: UserResponse }) {
  const store = useStore();
  const [notice, setNotice] = useState<string>();
  const [round, setRound] = useState(0);
  const form = useSubmit(
    resetSchema,
    (body) => updateUser(user.id, body),
    (saved) => {
      store(saved);
      setRound((value) => value + 1);
      setNotice(
        `The password is reset. Hand the new temporary password to ${saved.displayName}; the account is unlocked and signed out on every device.`
      );
    }
  );

  return (
    <Section
      title="Reset Password"
      description="For a forgotten password. It also ends a lockout and signs the account out everywhere."
    >
      <form
        key={round}
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          const password = new FormData(event.currentTarget).get("password");
          void form.submit({ password: typeof password === "string" ? password : "" });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <TemporaryPassword label="New temporary password" error={form.errors.password} />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            Reset password
          </Button>
        </div>
      </form>
    </Section>
  );
}

function Remove({ user }: { user: UserResponse }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const remove = useMutation({
    mutationFn: () => deleteUser(user.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.user(user.id) });
      void client.invalidateQueries({ queryKey: keys.users });
      navigate(path.users, { replace: true });
    },
  });

  return (
    <Section title="Delete User" description="Deletes the account and everything it owns.">
      <Button variant="danger" onClick={() => setOpen(true)}>
        Delete user
      </Button>
      <ConfirmDialog
        open={open}
        title={`Delete @${user.username}?`}
        confirm="Delete user"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => {
          setOpen(false);
          remove.reset();
        }}
      >
        <p>
          {`${user.displayName}'s chats, projects, files, schedules, providers, and tool servers are deleted with the account. This cannot be undone.`}
        </p>
        <p>To keep their data, disable the account instead.</p>
      </ConfirmDialog>
    </Section>
  );
}

/** Settings, Users, one account (administrators only): profile, access, password reset, delete. */
export function UserDetails({ id }: { id: string }) {
  const me = useUser();
  const account = useQuery({
    queryKey: keys.user(id),
    queryFn: ({ signal }) => fetchUser(id, signal),
  });
  const user = account.data;
  const self = user?.id === me.id;

  return (
    <SettingsPage
      title="User"
      heading={user?.displayName}
      description={
        user ? (
          <span className="flex flex-wrap items-center gap-2">
            {`@${user.username}`}
            {self ? <Badge tone="ok">You</Badge> : null}
            {accountBadges(user).map((badge) => (
              <Badge key={badge.label} tone={badge.tone}>
                {badge.label}
              </Badge>
            ))}
          </span>
        ) : undefined
      }
      back={BACK}
    >
      <QueryState query={account} back={BACK}>
        {user ? (
          <>
            {self ? (
              <p role="note" className="form-warning">
                This is your account. Saving a change here signs you out on every device, this one
                included. Change your own name and password in Settings, Account.
              </p>
            ) : null}
            <Profile user={user} />
            <Access user={user} />
            <Reset user={user} />
            <Remove user={user} />
          </>
        ) : null}
      </QueryState>
    </SettingsPage>
  );
}
