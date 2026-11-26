import Home from "@web/routes/home/page";
import NotFoundPage from "@web/routes/not-found/page";
import type { ComponentType } from "react";
import { Route, Routes } from "react-router-dom";
import "@web/assets/styles/main.css";
import path from "@web/routes/path";

type RouteConfig = {
  path: string;
  element: ComponentType;
};

export const routes: RouteConfig[] = [
  {
    path: path.home,
    element: Home,
  },
  {
    path: path.notFound,
    element: NotFoundPage,
  },
];

export function AppRouter() {
  return (
    <Routes>
      {routes.map(({ path: routePath, element: Element }) => (
        <Route key={routePath} path={routePath} element={<Element />} />
      ))}
    </Routes>
  );
}
