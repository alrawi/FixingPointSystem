// SPDX-License-Identifier: GPL-3.0-only

import { QueryClient, QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router";
import { ApiError } from "./api/client";
import { SessionProvider, useSession } from "./auth/session";
import { SignInPage } from "./auth/SignInPage";
import { AppLayout } from "./layout/AppLayout";
import { CapabilitiesPage } from "./pages/CapabilitiesPage";
import { ModuleDetailPage } from "./pages/ModuleDetailPage";
import { ModulesPage } from "./pages/ModulesPage";
import { NotFoundPage } from "./pages/NotFoundPage";

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
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}

export interface AppProps {
  /** Base URL of the controller API, `/v1` by default. */
  baseUrl?: string;
  queryClient?: QueryClient;
}

/** The whole UI. */
export function App({ baseUrl, queryClient }: AppProps) {
  const [client] = useState(() => queryClient ?? createQueryClient());
  return (
    <QueryClientProvider client={client}>
      <SessionProvider {...(baseUrl === undefined ? {} : { baseUrl })}>
        <BrowserRouter>
          <Routed />
        </BrowserRouter>
      </SessionProvider>
    </QueryClientProvider>
  );
}
