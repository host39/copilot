import { ErrorBoundary } from "@web/components/Error/ErrorBoundary";
import { Layout } from "@web/components/Layout/Base";
import { DefaultLoading } from "@web/components/Suspense/DefaultLoading";
import { AppRouter } from "@web/routes";
import React, { Suspense } from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";

const rootEl = document.getElementById("root");
if (rootEl) {
  const root = ReactDOM.createRoot(rootEl);
  root.render(
    <React.StrictMode>
      <BrowserRouter>
        <ErrorBoundary>
          <Layout>
            <Suspense fallback={<DefaultLoading />}>
              <AppRouter />
            </Suspense>
          </Layout>
        </ErrorBoundary>
      </BrowserRouter>
    </React.StrictMode>
  );
}
