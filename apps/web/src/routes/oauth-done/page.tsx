import { ApiError } from "@web/api/client";
import { confirmSignIn } from "@web/api/routes/tool-servers";
import { useToken } from "@web/auth/session";
import Button from "@web/components/Button/Button";
import { CenteredPage } from "@web/components/Layout/CenteredPage";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { link } from "@web/routes/path";
import {
  confirmationCode,
  doneMessage,
  type SignInResult,
} from "@web/routes/settings/tool-server/sign-in";
import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

/** The uuid shape of a server id, so only an id ever becomes a link. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * `/oauth/done`: where a tool server sign-in sends the browser back (`?result=&server=`, and
 * `#<code>` when the sign-in came back and waits for its owner). It needs no session: in the
 * native apps it opens in the system browser, which is not signed in, and shows the code to type
 * in the app. A browser that is signed in to Copilot sends the code itself; the server takes it
 * only from the account that started the sign-in.
 */
export default function SignInDonePage() {
  const [params] = useSearchParams();
  const { hash } = useLocation();
  const navigate = useNavigate();
  const token = useToken();
  const server = params.get("server");
  const id = server && UUID.test(server) ? server : null;
  const code = params.get("result") === "confirm" ? confirmationCode(hash) : null;
  const [ended, setEnded] = useState<SignInResult>();
  const sent = useRef(false);
  useEffect(() => {
    if (!code || !id || !token || sent.current) return;
    sent.current = true;
    confirmSignIn(id, code).then(
      () => setEnded("signed-in"),
      (error: unknown) =>
        setEnded(error instanceof ApiError && error.status === 404 ? "other-account" : "failed")
    );
  }, [code, id, token]);
  const result = ended ?? (code ? "confirm" : params.get("result"));
  const message = doneMessage(result === "confirm" && !code ? "failed" : result);
  const finishing = result === "confirm" && token !== null && id !== null;

  return (
    <CenteredPage>
      <PageTitle page={message.title} />
      <h1 className="page-heading">{finishing ? "Finishing the sign-in" : message.title}</h1>
      {finishing ? null : <p className="mt-3 text-sm text-muted sm:text-base">{message.body}</p>}
      {result === "confirm" && !finishing && code ? (
        <p className="mt-6 text-sm text-muted">
          Code:{" "}
          <output className="select-all font-mono text-3xl tracking-widest text-fg">{code}</output>
        </p>
      ) : null}
      {id && result !== "other-account" ? (
        <div className="mt-6 flex justify-center">
          <Button
            onClick={() =>
              navigate(result === "confirm" ? `${link.toolServer(id)}?code` : link.toolServer(id))
            }
          >
            Open the tool server
          </Button>
        </div>
      ) : null}
    </CenteredPage>
  );
}
