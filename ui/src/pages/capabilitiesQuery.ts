// SPDX-License-Identifier: GPL-3.0-only

import { useQuery } from "@tanstack/react-query";
import { fetchCapabilities } from "../api/client";
import { useApiClient } from "../auth/session";

/** Query key of GET /capabilities. */
export const capabilitiesKey = ["capabilities"] as const;

/** Loads the capabilities of the controller. */
export function useCapabilities() {
  const client = useApiClient();
  return useQuery({
    queryKey: capabilitiesKey,
    queryFn: ({ signal }) => fetchCapabilities(client, signal),
  });
}
