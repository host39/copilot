import { fetchHealth } from "@web/api/routes/health";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { API_URL } from "@web/config";
import { useEffect, useState } from "react";

const HomePage = () => {
  const [status, setStatus] = useState("checking...");

  useEffect(() => {
    let cancelled = false;

    fetchHealth()
      .then((data) => {
        if (!cancelled) {
          setStatus(data.status);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setStatus("unreachable");
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const isHealthy = status === "healthy";

  return (
    <div className="flex min-h-[calc(100vh-4rem)] flex-col items-center justify-center gap-6 p-8">
      <PageTitle />
      <div className="w-full max-w-md rounded-xl border border-gray-200 bg-white p-8 shadow-sm">
        <h1 className="text-2xl font-bold">Copilot</h1>
        <p className="mt-2 text-sm text-gray-600">
          A starter with a Rust (axum) API and a React web app.
        </p>

        <div className="mt-6 flex items-center justify-between rounded-lg bg-gray-50 px-4 py-3">
          <span className="text-sm font-medium text-gray-700">API health</span>
          <span
            className={`inline-flex items-center gap-2 text-sm font-semibold ${
              isHealthy ? "text-emerald-600" : "text-amber-600"
            }`}
          >
            <span
              className={`h-2.5 w-2.5 rounded-full ${
                isHealthy ? "bg-emerald-500" : "bg-amber-500"
              }`}
              aria-hidden
            />
            {status}
          </span>
        </div>

        <p className="mt-4 text-xs text-gray-500">
          Fetched from <code>{API_URL}/health</code>
        </p>
      </div>
    </div>
  );
};

export default HomePage;
