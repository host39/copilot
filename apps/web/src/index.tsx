import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "@web/api/query";
import { ErrorBoundary } from "@web/components/Error/ErrorBoundary";
import { DefaultLoading } from "@web/components/Suspense/DefaultLoading";
import { log } from "@web/logging";
import { installNativeTransport } from "@web/native";
import { AppRouter } from "@web/routes";
import { applyTheme, readTheme } from "@web/theme";
import React, { Suspense } from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";

async function start(root: HTMLElement): Promise<void> {
  applyTheme(readTheme());
  try {
    await installNativeTransport();
  } catch (error) {
    log("error", "web.native.transport.failed", { reason: String(error) });
  }
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <ErrorBoundary>
            <Suspense fallback={<DefaultLoading />}>
              <AppRouter />
            </Suspense>
          </ErrorBoundary>
        </BrowserRouter>
      </QueryClientProvider>
    </React.StrictMode>
  );
}

const root = document.getElementById("root");
if (root) void start(root);
