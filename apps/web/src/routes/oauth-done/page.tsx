import Button from "@web/components/Button/Button";
import { CenteredPage } from "@web/components/Layout/CenteredPage";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { link } from "@web/routes/path";
import { doneMessage } from "@web/routes/settings/tool-server/sign-in";
import { useNavigate, useSearchParams } from "react-router-dom";

/** The uuid shape of a server id, so only an id ever becomes a link. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * `/oauth/done`: where a tool server sign-in sends the browser back (`?result=&server=`). It
 * needs no session: in the native apps it opens in the system browser, which is not signed in.
 */
export default function SignInDonePage() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const message = doneMessage(params.get("result"));
  const server = params.get("server");

  return (
    <CenteredPage>
      <PageTitle page={message.title} />
      <h1 className="page-heading">{message.title}</h1>
      <p className="mt-3 text-sm text-muted sm:text-base">{message.body}</p>
      {server && UUID.test(server) ? (
        <div className="mt-6 flex justify-center">
          <Button onClick={() => navigate(link.toolServer(server))}>Open the tool server</Button>
        </div>
      ) : null}
    </CenteredPage>
  );
}
