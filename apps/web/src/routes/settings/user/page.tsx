import { AddUser } from "@web/routes/settings/user/add";
import { UserDetails } from "@web/routes/settings/user/details";
import { useParams } from "react-router-dom";

/** `/settings/users/new` creates an account; `/settings/users/:userId` manages one. */
export default function UserPage() {
  const { userId } = useParams();
  return userId ? <UserDetails key={userId} id={userId} /> : <AddUser />;
}
