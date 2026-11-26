import { isAdmin, useUser } from "@web/auth/session";
import path from "@web/routes/path";
import { NavLink, Outlet } from "react-router-dom";

/** Settings: a section list beside the section on wide windows, tabs above it on phones. */
export default function SettingsLayout() {
  const user = useUser();
  const sections = [
    { to: path.account, label: "Account" },
    { to: path.providers, label: "Providers" },
    { to: path.toolServers, label: "Tool servers" },
    ...(isAdmin(user) ? [{ to: path.users, label: "Users" }] : []),
  ];

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col md:flex-row md:gap-6 md:px-6 md:py-8">
      <nav
        aria-label="Settings"
        className="flex shrink-0 gap-1 overflow-x-auto border-b border-line px-2 py-2 md:w-48 md:flex-col md:border-0 md:p-0"
      >
        <h1 className="sr-only">Settings</h1>
        {sections.map((section) => (
          <NavLink key={section.to} to={section.to} className="nav-item shrink-0">
            {section.label}
          </NavLink>
        ))}
      </nav>
      <div className="min-w-0 flex-1">
        <Outlet />
      </div>
    </div>
  );
}
