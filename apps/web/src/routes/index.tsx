// The route table: every path in `path.tsx` and its page. Pages load on demand, so a screen's
// code (and later the Markdown renderer) stays off the sign-in screens.

import { GuestOnly, RequireAdmin, RequireServer, RequireSession } from "@web/auth/guards";
import { AppShell } from "@web/components/Layout/AppShell";
import { isNative } from "@web/config";
import path from "@web/routes/path";
import { lazy } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import "@web/assets/styles/main.css";

const Login = lazy(() => import("@web/routes/auth/login/page"));
const Register = lazy(() => import("@web/routes/auth/register/page"));
const Setup = lazy(() => import("@web/routes/auth/setup/page"));
const Connect = lazy(() => import("@web/routes/connect/page"));
const Welcome = lazy(() => import("@web/routes/welcome/page"));

const Home = lazy(() => import("@web/routes/home/page"));
const Chat = lazy(() => import("@web/routes/chat/page"));
const Search = lazy(() => import("@web/routes/search/page"));
const Scheduled = lazy(() => import("@web/routes/scheduled/page"));
const Schedule = lazy(() => import("@web/routes/schedule/page"));
const Projects = lazy(() => import("@web/routes/projects/page"));
const Project = lazy(() => import("@web/routes/project/page"));

const Settings = lazy(() => import("@web/routes/settings/layout"));
const Account = lazy(() => import("@web/routes/settings/account/page"));
const Providers = lazy(() => import("@web/routes/settings/providers/page"));
const Provider = lazy(() => import("@web/routes/settings/provider/page"));
const ToolServers = lazy(() => import("@web/routes/settings/tool-servers/page"));
const ToolServer = lazy(() => import("@web/routes/settings/tool-server/page"));
const Users = lazy(() => import("@web/routes/settings/users/page"));
const User = lazy(() => import("@web/routes/settings/user/page"));

const NotFound = lazy(() => import("@web/routes/not-found/page"));

export function AppRouter() {
  return (
    <Routes>
      <Route
        path={path.connect}
        element={isNative() ? <Connect /> : <Navigate to={path.home} replace />}
      />
      <Route element={<RequireServer />}>
        <Route element={<GuestOnly />}>
          <Route path={path.login} element={<Login />} />
          <Route path={path.register} element={<Register />} />
          <Route path={path.setup} element={<Setup />} />
        </Route>

        <Route element={<RequireSession />}>
          <Route path={path.welcome} element={<Welcome />} />
          <Route element={<AppShell />}>
            <Route path={path.home} element={<Home />} />
            <Route path={path.chat} element={<Chat />} />
            <Route path={path.search} element={<Search />} />
            <Route path={path.scheduled} element={<Scheduled />} />
            <Route path={path.scheduleNew} element={<Schedule />} />
            <Route path={path.schedule} element={<Schedule />} />
            <Route path={path.projects} element={<Projects />} />
            <Route path={path.projectNew} element={<Project />} />
            <Route path={path.project} element={<Project />} />

            <Route path={path.settings} element={<Settings />}>
              <Route index element={<Navigate to={path.account} replace />} />
              <Route path={path.account} element={<Account />} />
              <Route path={path.providers} element={<Providers />} />
              <Route path={path.providerNew} element={<Provider />} />
              <Route path={path.provider} element={<Provider />} />
              <Route path={path.toolServers} element={<ToolServers />} />
              <Route path={path.toolServerNew} element={<ToolServer />} />
              <Route path={path.toolServer} element={<ToolServer />} />
              <Route element={<RequireAdmin />}>
                <Route path={path.users} element={<Users />} />
                <Route path={path.userNew} element={<User />} />
                <Route path={path.user} element={<User />} />
              </Route>
            </Route>

            <Route path={path.notFound} element={<NotFound />} />
          </Route>
        </Route>
      </Route>
    </Routes>
  );
}
