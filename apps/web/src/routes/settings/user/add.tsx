import { createUserRequestSchema, type UserResponse, type UserRole } from "@copilot/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { createUser } from "@web/api/routes/users";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { RolePicker } from "@web/components/Settings/RolePicker";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { TemporaryPassword } from "@web/components/Settings/TemporaryPassword";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path, { link } from "@web/routes/path";
import { useState } from "react";
import { Link } from "react-router-dom";

const BACK = { to: path.users, label: "Users" };

/** The new account and the password to hand over, shown once after creating it. */
function Created({
  user,
  password,
  onAnother,
}: {
  user: UserResponse;
  password: string;
  onAnother: () => void;
}) {
  return (
    <div className="card-panel space-y-4">
      <p role="status" className="text-sm">
        {`${user.displayName} can now sign in as @${user.username} with this temporary password. Hand it over in person or another safe way; they can change it in Settings, Account.`}
      </p>
      <p className="break-all rounded-lg bg-surface px-3 py-2.5 font-mono text-base select-all">
        {password}
      </p>
      <div className="flex flex-wrap gap-2">
        <Link to={link.user(user.id)} className="button-link">
          Open account
        </Link>
        <Button variant="secondary" onClick={onAnother}>
          Add another user
        </Button>
      </div>
    </div>
  );
}

/** Settings, Users, Add (administrators only): an account with a temporary password and a role. */
export function AddUser() {
  const client = useQueryClient();
  const [role, setRole] = useState<UserRole>("user");
  const [created, setCreated] = useState<{ user: UserResponse; password: string }>();
  const [round, setRound] = useState(0);
  const form = useSubmit(
    createUserRequestSchema,
    async (body) => ({ user: await createUser(body), password: body.temporaryPassword }),
    (result) => {
      void client.invalidateQueries({ queryKey: keys.users });
      setCreated(result);
    }
  );

  return (
    <SettingsPage title="Add User" back={BACK}>
      {created ? (
        <Created
          user={created.user}
          password={created.password}
          onAnother={() => {
            setCreated(undefined);
            setRole("user");
            setRound((value) => value + 1);
          }}
        />
      ) : (
        <form
          key={round}
          noValidate
          className="card-panel space-y-6"
          onSubmit={(event) => {
            event.preventDefault();
            const data = new FormData(event.currentTarget);
            const password = data.get("password");
            void form.submit({
              username: text(data, "username"),
              displayName: text(data, "displayName"),
              temporaryPassword: typeof password === "string" ? password : "",
              role,
            });
          }}
        >
          <FormAlert>{form.alert}</FormAlert>
          <TextField
            label="Name"
            name="displayName"
            autoComplete="off"
            error={form.errors.displayName}
          />
          <TextField
            label="Username"
            name="username"
            autoComplete="off"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            hint="3 to 32 letters, digits, or underscores. They sign in with it."
            error={form.errors.username}
          />
          <TemporaryPassword label="Temporary password" error={form.errors.temporaryPassword} />
          <RolePicker value={role} onChange={setRole} />
          <div className="flex justify-end">
            <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
              Create account
            </Button>
          </div>
        </form>
      )}
    </SettingsPage>
  );
}
