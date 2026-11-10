// SPDX-License-Identifier: GPL-3.0-only

import { useQuery } from "@tanstack/react-query";
import { fetchRunReport, isFinalRunState } from "../api/client";
import { useApiClient } from "../auth/session";
import { useRunSettings } from "../runs/useRunMetrics";

/** Query key of the report of one run, GET /runs/{id}/report. */
export function reportKey(id: string) {
  return ["runs", id, "report"] as const;
}

/**
 * Loads the report of one run. The report of a run that is still active changes, so it is read
 * again every few seconds until the run is final. An empty id loads nothing.
 */
export function useRunReport(id: string) {
  const client = useApiClient();
  const { pollMs } = useRunSettings();
  return useQuery({
    queryKey: reportKey(id),
    queryFn: ({ signal }) => fetchRunReport(client, id, signal),
    enabled: id !== "",
    staleTime: 0,
    refetchInterval: (query) => {
      const run = query.state.data?.run;
      return run !== undefined && isFinalRunState(run.state) ? false : pollMs * 3;
    },
  });
}
