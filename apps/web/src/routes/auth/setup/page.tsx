import { registerRequestSchema } from "@copilot/contracts";
import { register } from "@web/api/routes/auth";
import { useSetupStatus, useSignIn } from "@web/auth/session";
import { AccountFields, accountInput } from "@web/components/Auth/AccountFields";
import { AuthCard } from "@web/components/Auth/AuthCard";
import { SubmitButton } from "@web/components/Auth/SubmitButton";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { Navigate, useNavigate } from "react-router-dom";
import { z } from "zod";

/** rustauth's register schema, with the setup token required. */
const schema = registerRequestSchema.extend({
  setupToken: z.string().min(1, "Enter the setup token from the server log."),
});

/** The first account, which becomes the administrator. It needs the server's setup token. */
export default function SetupPage() {
  const setup = useSetupStatus();
  const signIn = useSignIn();
  const navigate = useNavigate();
  const form = useSubmit(schema, register, (response) => {
    signIn(response);
    navigate(path.welcome, { replace: true });
  });

  if (setup.isPending) return <PageLoading />;
  if (setup.data && !setup.data.needsSetup) return <Navigate to={path.login} replace />;

  return (
    <AuthCard
      title="Set Up Copilot"
      description={
        <>
          Create the administrator account. The setup token is in the server log (look for{" "}
          <code className="text-fg">rustauth.setup.token-issued</code>) or in{" "}
          <code className="text-fg">AUTH_SETUP_TOKEN</code>.
        </>
      }
    >
      <form
        method="post"
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          void form.submit({ ...accountInput(data), setupToken: text(data, "setupToken") });
        }}
      >
        <FormAlert>
          {form.alert ?? (setup.isError ? "The server cannot be reached." : null)}
        </FormAlert>
        <TextField
          label="Setup token"
          name="setupToken"
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          required
          error={form.errors.setupToken}
        />
        <AccountFields errors={form.errors} />
        <SubmitButton busy={form.busy} wait={form.wait}>
          Create administrator
        </SubmitButton>
      </form>
    </AuthCard>
  );
}
