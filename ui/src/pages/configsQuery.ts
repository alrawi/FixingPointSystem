// SPDX-License-Identifier: GPL-3.0-only

import { useQuery } from "@tanstack/react-query";
import { fetchConfig, fetchConfigs } from "../api/client";
import { useApiClient } from "../auth/session";

/** Query key of the stored configuration list, GET /configs over every page. */
export const configsKey = ["configs"] as const;

/** Query key of one stored configuration, GET /configs/{id}. */
export function configKey(id: string) {
  return ["configs", id] as const;
}

/** Loads every stored configuration, newest first. */
export function useConfigs() {
  const client = useApiClient();
  return useQuery({
    queryKey: configsKey,
    queryFn: ({ signal }) => fetchConfigs(client, signal),
  });
}

/** Loads one stored configuration. */
export function useConfig(id: string) {
  const client = useApiClient();
  return useQuery({
    queryKey: configKey(id),
    queryFn: ({ signal }) => fetchConfig(client, id, signal),
  });
}
