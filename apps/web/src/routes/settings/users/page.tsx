import type { UserResponse } from "@copilot/contracts";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchUsers, USERS_PAGE } from "@web/api/routes/users";
import { useUser } from "@web/auth/session";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { QueryState } from "@web/components/Settings/QueryState";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import path, { link } from "@web/routes/path";
import { accountBadges } from "@web/routes/settings/user/accounts";
import { useState } from "react";
import { LuChevronRight, LuPlus } from "react-icons/lu";
import { Link } from "react-router-dom";

function Row({ user, self }: { user: UserResponse; self: boolean }) {
  return (
    <li>
      <Link to={link.user(user.id)} className="list-row">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span className="break-words">{user.displayName}</span>
            {self ? <Badge tone="ok">You</Badge> : null}
            {accountBadges(user).map((badge) => (
              <Badge key={badge.label} tone={badge.tone}>
                {badge.label}
              </Badge>
            ))}
          </p>
          <p className="truncate text-sm text-muted">{`@${user.username}`}</p>
        </div>
        <LuChevronRight aria-hidden className="shrink-0 text-muted" />
      </Link>
    </li>
  );
}

/** Settings, Users (administrators only): every account, 100 per page. */
export default function UsersPage() {
  const me = useUser();
  const [offset, setOffset] = useState(0);
  const users = useQuery({
    queryKey: keys.usersPage(offset),
    queryFn: ({ signal }) => fetchUsers(offset, signal),
    placeholderData: keepPreviousData,
  });
  const page = users.data;
  const total = page?.total ?? 0;

  return (
    <SettingsPage
      title="Users"
      description="Accounts on this server. New accounts sign in with a temporary password you hand over."
      actions={
        <Link to={path.userNew} className="button-link">
          <LuPlus aria-hidden />
          Add user
        </Link>
      }
    >
      <QueryState query={users}>
        <ul className="list-panel">
          {(page?.items ?? []).map((user) => (
            <Row key={user.id} user={user} self={user.id === me.id} />
          ))}
        </ul>
        {total > USERS_PAGE ? (
          <nav aria-label="Pages" className="flex flex-wrap items-center justify-between gap-2">
            <Button
              variant="secondary"
              disabled={offset === 0 || users.isFetching}
              onClick={() => setOffset(Math.max(0, offset - USERS_PAGE))}
            >
              Previous
            </Button>
            <span className="text-sm text-muted">
              {`${offset + 1} to ${Math.min(offset + USERS_PAGE, total)} of ${total}`}
            </span>
            <Button
              variant="secondary"
              disabled={offset + USERS_PAGE >= total || users.isFetching}
              onClick={() => setOffset(offset + USERS_PAGE)}
            >
              Next
            </Button>
          </nav>
        ) : null}
      </QueryState>
    </SettingsPage>
  );
}
