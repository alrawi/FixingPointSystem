// SPDX-License-Identifier: GPL-3.0-only

import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { ApiError, fetchModule, fetchModules } from "../api/client";
import { useApiClient, useSession } from "../auth/session";

/** Query key of the full module list, GET /modules over every page. */
export const modulesKey = ["modules"] as const;

/** Query key of one module manifest, GET /modules/{name}/{version}. */
export function moduleKey(name: string, version: string) {
  return ["modules", name, version] as const;
}

/** Loads every installed module. */
export function useModules() {
  const client = useApiClient();
  return useQuery({
    queryKey: modulesKey,
    queryFn: ({ signal }) => fetchModules(client, { signal }),
  });
}

/** Loads the manifest of one module. */
export function useModule(name: string, version: string) {
  const client = useApiClient();
  return useQuery({
    queryKey: moduleKey(name, version),
    queryFn: ({ signal }) => fetchModule(client, name, version, signal),
  });
}

/** Signs out with a notice when a query failed because the controller rejects the token. */
export function useSignOutWhenUnauthorized(error: unknown): boolean {
  const { signOut } = useSession();
  const unauthorized = error instanceof ApiError && error.isUnauthorized;
  useEffect(() => {
    if (unauthorized) signOut("The controller no longer accepts the token. Sign in again.");
  }, [unauthorized, signOut]);
  return unauthorized;
}
