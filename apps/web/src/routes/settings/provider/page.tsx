import { AddProvider } from "@web/routes/settings/provider/add";
import { ProviderDetails } from "@web/routes/settings/provider/details";
import { useParams } from "react-router-dom";

/** `/settings/providers/new` adds a connection; `/settings/providers/:providerId` shows one. */
export default function ProviderPage() {
  const { providerId } = useParams();
  return providerId ? <ProviderDetails key={providerId} id={providerId} /> : <AddProvider />;
}
