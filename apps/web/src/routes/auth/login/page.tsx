import { loginRequestSchema } from "@copilot/contracts";
import { login } from "@web/api/routes/auth";
import type { FromState } from "@web/auth/guards";
import { useSetupStatus, useSignIn, useToken } from "@web/auth/session";
import { AuthCard } from "@web/components/Auth/AuthCard";
import { SubmitButton } from "@web/components/Auth/SubmitButton";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { landing } from "@web/routes/welcome/setup";
import { Link, Navigate, useLocation, useNavigate } from "react-router-dom";
import { z } from "zod";

/** rustauth's login schema, with messages for empty fields. */
const schema = loginRequestSchema.extend({
  username: z.string().min(1, "Enter your username."),
  password: z.string().min(1, "Enter your password."),
});

export default function LoginPage() {
  const setup = useSetupStatus();
  const signedIn = useToken() !== null;
  const signIn = useSignIn();
  const navigate = useNavigate();
  const from = (useLocation().state as FromState | null)?.from;
  const form = useSubmit(schema, login, (response) => {
    signIn(response);
    // A user without a provider connection gets the first-run setup.
    void landing(from ?? path.home).then((target) => navigate(target, { replace: true }));
  });

  if (setup.data?.needsSetup && !signedIn) return <Navigate to={path.setup} replace />;

  return (
    <AuthCard
      title="Sign In"
      footer={
        setup.data?.registrationOpen ? (
          <>
            New here?{" "}
            <Link to={path.register} className="text-link">
              Create an account
            </Link>
          </>
        ) : null
      }
    >
      <form
        method="post"
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          void form.submit({
            username: text(data, "username"),
            password: data.get("password") ?? "",
          });
        }}
      >
        <FormAlert>
          {form.alert ?? (setup.isError ? "The server cannot be reached." : null)}
        </FormAlert>
        <TextField
          label="Username"
          name="username"
          autoComplete="username"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          required
          error={form.errors.username}
        />
        <TextField
          label="Password"
          name="password"
          type="password"
          autoComplete="current-password"
          required
          error={form.errors.password}
        />
        <SubmitButton busy={form.busy} wait={form.wait}>
          Sign in
        </SubmitButton>
      </form>
    </AuthCard>
  );
}
