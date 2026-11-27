import {
  type SessionResponse,
  type UserResponse,
  updateProfileRequestSchema,
  updateSettingsRequestSchema,
} from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import {
  changePassword,
  fetchSessions,
  revokeOtherSessions,
  revokeSession,
  updateProfile,
} from "@web/api/routes/auth";
import { keys } from "@web/api/routes/keys";
import { fetchSettings, updateSettings } from "@web/api/routes/settings";
import { useSignOut, useUser } from "@web/auth/session";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { TextArea } from "@web/components/Form/TextArea";
import { TextField } from "@web/components/Form/TextField";
import { QueryState } from "@web/components/Settings/QueryState";
import { Section } from "@web/components/Settings/Section";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { ThemePicker } from "@web/components/Settings/ThemePicker";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import { deviceName, sortSessions } from "@web/routes/settings/account/devices";
import { passwordSchema } from "@web/routes/settings/account/password";
import { ago, dateTime } from "@web/routes/settings/format";
import { useState } from "react";
import { LuLaptop, LuSmartphone } from "react-icons/lu";

function Profile({ user }: { user: UserResponse }) {
  const client = useQueryClient();
  const [notice, setNotice] = useState<string>();
  const form = useSubmit(updateProfileRequestSchema, updateProfile, (saved) => {
    client.setQueriesData<UserResponse>({ queryKey: ["auth", "me"] }, saved);
    setNotice("Saved.");
  });

  return (
    <Section title="Profile" description={`You sign in as @${user.username}.`}>
      <form
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          void form.submit({ displayName: text(new FormData(event.currentTarget), "displayName") });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <TextField
          label="Display name"
          name="displayName"
          autoComplete="name"
          defaultValue={user.displayName}
          required
          error={form.errors.displayName}
        />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            Save name
          </Button>
        </div>
      </form>
    </Section>
  );
}

function Password({ username }: { username: string }) {
  const client = useQueryClient();
  const [notice, setNotice] = useState<string>();
  const [round, setRound] = useState(0);
  const form = useSubmit(passwordSchema, changePassword, () => {
    setRound((value) => value + 1);
    setNotice("Password changed. Your other devices were signed out.");
    void client.invalidateQueries({ queryKey: keys.sessions });
  });

  return (
    <Section
      title="Password"
      description="Changing it signs out every other device. This one stays signed in."
    >
      <form
        key={round}
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          const data = new FormData(event.currentTarget);
          void form.submit({
            current: data.get("current") ?? "",
            new: data.get("new") ?? "",
            confirm: data.get("confirm") ?? "",
          });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        {/* Password managers pair the new password with this account. */}
        <input type="text" autoComplete="username" value={username} hidden readOnly />
        <TextField
          label="Current password"
          name="current"
          type="password"
          autoComplete="current-password"
          required
          error={form.errors.current}
        />
        <TextField
          label="New password"
          name="new"
          type="password"
          autoComplete="new-password"
          required
          hint="At least 12 characters."
          error={form.errors.new}
        />
        <TextField
          label="Repeat the new password"
          name="confirm"
          type="password"
          autoComplete="new-password"
          required
          error={form.errors.confirm}
        />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {form.wait > 0 ? `Try again in ${form.wait} s` : "Change password"}
          </Button>
        </div>
      </form>
    </Section>
  );
}

function Instructions() {
  const settings = useQuery({
    queryKey: keys.settings,
    queryFn: ({ signal }) => fetchSettings(signal),
  });
  const client = useQueryClient();
  const [notice, setNotice] = useState<string>();
  const form = useSubmit(updateSettingsRequestSchema, updateSettings, (saved) => {
    client.setQueryData(keys.settings, saved);
    setNotice("Saved.");
  });

  return (
    <Section
      title="Personal Instructions"
      description="Sent with every chat, before a project's instructions. Tell the model about yourself or how you like answers."
    >
      <QueryState query={settings}>
        <form
          noValidate
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            setNotice(undefined);
            const value = new FormData(event.currentTarget).get("instructions");
            void form.submit({ instructions: typeof value === "string" ? value : "" });
          }}
        >
          <FormAlert>{form.alert}</FormAlert>
          <FormNotice>{notice}</FormNotice>
          <TextArea
            label="Instructions"
            name="instructions"
            rows={6}
            defaultValue={settings.data?.instructions ?? ""}
            placeholder="For example: I am a software developer. Keep answers short."
            error={form.errors.instructions}
          />
          <div className="flex justify-end">
            <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
              Save instructions
            </Button>
          </div>
        </form>
      </QueryState>
    </Section>
  );
}

function Device({
  session,
  busy,
  onSignOut,
}: {
  session: SessionResponse;
  busy: boolean;
  onSignOut: () => void;
}) {
  const name = deviceName(session.userAgent);
  const Icon = /Android|iOS/.test(name) ? LuSmartphone : LuLaptop;
  return (
    <li className="flex flex-wrap items-center gap-3 py-3">
      <Icon aria-hidden className="size-5 shrink-0 text-muted" />
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
          <span className="break-words">{name}</span>
          {session.current ? <Badge tone="accent">This device</Badge> : null}
        </p>
        <p className="text-sm text-muted">
          {session.current ? "Active now" : `Last used ${ago(session.lastUsedAt)}`}
          {session.ip ? ` · ${session.ip}` : ""}
          {` · Signed in ${dateTime(session.createdAt)}`}
        </p>
      </div>
      <Button
        variant="secondary"
        busy={busy}
        onClick={onSignOut}
        aria-label={`Sign out ${session.current ? "this device" : name}`}
      >
        Sign out
      </Button>
    </li>
  );
}

function Devices() {
  const client = useQueryClient();
  const signOut = useSignOut();
  const sessions = useQuery({
    queryKey: keys.sessions,
    queryFn: ({ signal }) => fetchSessions(signal),
  });
  const [notice, setNotice] = useState<string>();
  const revoke = useMutation({
    mutationFn: revokeSession,
    onSuccess: () => {
      setNotice("Signed out.");
      return client.invalidateQueries({ queryKey: keys.sessions });
    },
  });
  const others = useMutation({
    mutationFn: revokeOtherSessions,
    onSuccess: () => {
      setNotice("Every other device was signed out.");
      return client.invalidateQueries({ queryKey: keys.sessions });
    },
  });
  const list = sortSessions(sessions.data ?? []);
  const failure = revoke.error ?? others.error;

  return (
    <Section
      title="Devices"
      description="Where your account is signed in. Signing a device out ends its session at once."
      actions={
        list.some((session) => !session.current) ? (
          <Button
            variant="secondary"
            busy={others.isPending}
            onClick={() => {
              setNotice(undefined);
              others.mutate();
            }}
          >
            Sign out other devices
          </Button>
        ) : null
      }
    >
      <QueryState query={sessions}>
        <FormAlert>{failure ? describeError(failure).message : null}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <ul className="divide-y divide-line">
          {list.map((session) => (
            <Device
              key={session.id}
              session={session}
              busy={revoke.isPending && revoke.variables === session.id}
              onSignOut={() => {
                setNotice(undefined);
                if (session.current) void signOut();
                else revoke.mutate(session.id);
              }}
            />
          ))}
        </ul>
      </QueryState>
    </Section>
  );
}

/** Settings, Account: profile, password, personal instructions, appearance, and devices. */
export default function AccountPage() {
  const user = useUser();
  return (
    <SettingsPage title="Account">
      <Profile user={user} />
      <Password username={user.username} />
      <Instructions />
      <Section title="Appearance">
        <ThemePicker />
      </Section>
      <Devices />
    </SettingsPage>
  );
}
