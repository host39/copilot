import { registerRequestSchema } from "@copilot/contracts";
import { register } from "@web/api/routes/auth";
import { useSetupStatus, useSignIn, useToken } from "@web/auth/session";
import { AccountFields, accountInput } from "@web/components/Auth/AccountFields";
import { AuthCard } from "@web/components/Auth/AuthCard";
import { SubmitButton } from "@web/components/Auth/SubmitButton";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { Link, Navigate, useNavigate } from "react-router-dom";

/** Sign-up, offered only while the server's registration is open. */
export default function RegisterPage() {
  const setup = useSetupStatus();
  const signedIn = useToken() !== null;
  const signIn = useSignIn();
  const navigate = useNavigate();
  const form = useSubmit(registerRequestSchema, register, (response) => {
    signIn(response);
    navigate(path.welcome, { replace: true });
  });

  if (setup.isPending) return <PageLoading />;
  if (setup.data?.needsSetup && !signedIn) return <Navigate to={path.setup} replace />;

  const signInLink = (
    <>
      Have an account?{" "}
      <Link to={path.login} className="text-link">
        Sign in
      </Link>
    </>
  );

  if (setup.data && !setup.data.registrationOpen) {
    return (
      <AuthCard title="Create Account" footer={signInLink}>
        <p className="text-sm text-muted">
          This server does not accept new accounts. Ask an administrator to create one for you.
        </p>
      </AuthCard>
    );
  }

  return (
    <AuthCard title="Create Account" footer={signInLink}>
      <form
        method="post"
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          void form.submit(accountInput(new FormData(event.currentTarget)));
        }}
      >
        <FormAlert>
          {form.alert ?? (setup.isError ? "The server cannot be reached." : null)}
        </FormAlert>
        <AccountFields errors={form.errors} />
        <SubmitButton busy={form.busy} wait={form.wait}>
          Create account
        </SubmitButton>
      </form>
    </AuthCard>
  );
}
