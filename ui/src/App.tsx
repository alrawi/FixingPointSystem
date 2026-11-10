// SPDX-License-Identifier: GPL-3.0-only

import { QueryClient, QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router";
import { ApiError } from "./api/client";
import { SessionProvider, useSession } from "./auth/session";
import { SignInPage } from "./auth/SignInPage";
import { AppLayout } from "./layout/AppLayout";
import { CapabilitiesPage } from "./pages/CapabilitiesPage";
import { ComparePage } from "./pages/ComparePage";
import { ConfigsPage } from "./pages/ConfigsPage";
import { ModuleDetailPage } from "./pages/ModuleDetailPage";
import { ModulesPage } from "./pages/ModulesPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { ReportPage } from "./pages/ReportPage";
import { RunPage } from "./pages/RunPage";
import { RunsPage } from "./pages/RunsPage";
import { DEFAULT_RUN_SETTINGS, RunSettingsContext, type RunSettings } from "./runs/useRunMetrics";

// The editor ships a YAML library and a schema validator, so it loads on demand and the
// initial bundle stays within its budget.
const NewConfigPage = lazy(() =>
  import("./pages/ConfigEditorPage").then((module) => ({ default: module.NewConfigPage })),
);
const StoredConfigPage = lazy(() =>
  import("./pages/ConfigEditorPage").then((module) => ({ default: module.StoredConfigPage })),
);

function EditorLoading() {
  return (
    <p role="status" aria-live="polite">
      Loading the editor
    </p>
  );
}

/**
 * True when a failed query may succeed on a second try. Client errors such as
 * a rejected token or a missing module do not change on their own, so only
 * timeouts, rate limits, server errors, and network failures are retried.
 */
export function isRetryable(error: unknown): boolean {
  if (!(error instanceof ApiError)) return true;
  return error.status >= 500 || error.status === 408 || error.status === 429;
}

/** Creates the query cache. Errors that cannot heal are not retried. */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 10_000,
        retry: (failures, error) => isRetryable(error) && failures < 2,
      },
    },
  });
}

function Routed() {
  const { token } = useSession();
  const queryClient = useQueryClient();
  // Drop cached responses of an ended session, so a new token never sees them.
  useEffect(() => {
    if (token === null) queryClient.clear();
  }, [token, queryClient]);

  if (token === null) return <SignInPage />;
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route index element={<CapabilitiesPage />} />
        <Route path="modules" element={<ModulesPage />} />
        <Route path="modules/:name/:version" element={<ModuleDetailPage />} />
        <Route path="configs" element={<ConfigsPage />} />
        <Route
          path="configs/new"
          element={
            <Suspense fallback={<EditorLoading />}>
              <NewConfigPage />
            </Suspense>
          }
        />
        <Route
          path="configs/:id"
          element={
            <Suspense fallback={<EditorLoading />}>
              <StoredConfigPage />
            </Suspense>
          }
        />
        <Route path="runs" element={<RunsPage />} />
        <Route path="runs/:id" element={<RunPage />} />
        <Route path="runs/:id/report" element={<ReportPage />} />
        <Route path="compare" element={<ComparePage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}

export interface AppProps {
  /** Base URL of the controller API, `/v1` by default. */
  baseUrl?: string;
  queryClient?: QueryClient;
  /** Timings of the run view, one second by default. Tests shorten them. */
  runSettings?: Partial<RunSettings>;
}

/** The whole UI. */
export function App({ baseUrl, queryClient, runSettings }: AppProps) {
  const [client] = useState(() => queryClient ?? createQueryClient());
  const settings = useMemo(() => ({ ...DEFAULT_RUN_SETTINGS, ...runSettings }), [runSettings]);
  return (
    <QueryClientProvider client={client}>
      <RunSettingsContext.Provider value={settings}>
        <SessionProvider {...(baseUrl === undefined ? {} : { baseUrl })}>
          <BrowserRouter>
            <Routed />
          </BrowserRouter>
        </SessionProvider>
      </RunSettingsContext.Provider>
    </QueryClientProvider>
  );
}
