import { TextField } from "@web/components/Form/TextField";

/** The display name, username, and new password fields of the setup and register forms. */
export function AccountFields({ errors }: { errors: Record<string, string> }) {
  return (
    <>
      <TextField
        label="Your name"
        name="displayName"
        autoComplete="name"
        required
        error={errors.displayName}
      />
      <TextField
        label="Username"
        name="username"
        autoComplete="username"
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        required
        hint="3 to 32 letters, digits, or underscores."
        error={errors.username}
      />
      <TextField
        label="Password"
        name="password"
        type="password"
        autoComplete="new-password"
        required
        hint="At least 12 characters."
        error={errors.password}
      />
    </>
  );
}

/** The account fields of a submitted form, as `RegisterRequest` input. */
export function accountInput(data: FormData) {
  const value = (name: string) => {
    const field = data.get(name);
    return typeof field === "string" ? field : "";
  };
  return {
    displayName: value("displayName"),
    username: value("username"),
    password: value("password"),
  };
}
