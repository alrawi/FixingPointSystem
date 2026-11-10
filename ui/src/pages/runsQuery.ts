// SPDX-License-Identifier: GPL-3.0-only

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchAgents,
  fetchRun,
  fetchRuns,
  isFinalRunState,
  startRun,
  stopRun,
  type Run,
} from "../api/client";
import { useApiClient } from "../auth/session";
import { useRunSettings } from "../runs/useRunMetrics";

/** Query key of the run list, GET /runs over every page. */
export const runsKey = ["runs"] as const;

/** Query key of one run, GET /runs/{id}. */
export function runKey(id: string) {
  return ["runs", id] as const;
}

/** Query key of the agent list, GET /agents. */
export const agentsKey = ["agents"] as const;

/** Loads the runs, newest first, and refreshes them while any run is still active. */
export function useRuns() {
  const client = useApiClient();
  const { pollMs } = useRunSettings();
  return useQuery({
    queryKey: runsKey,
    queryFn: ({ signal }) => fetchRuns(client, { signal }),
    // A run changes state without anyone touching the list, so every visit reads it again.
    staleTime: 0,
    refetchInterval: (query) =>
      query.state.data?.some((run) => !isFinalRunState(run.state)) === true ? pollMs * 3 : false,
  });
}

/** Loads one run and polls it while it is active, so state changes show within a second. */
export function useRun(id: string) {
  const client = useApiClient();
  const { pollMs } = useRunSettings();
  return useQuery({
    queryKey: runKey(id),
    queryFn: ({ signal }) => fetchRun(client, id, signal),
    refetchInterval: (query) => {
      const run = query.state.data;
      return run !== undefined && isFinalRunState(run.state) ? false : pollMs;
    },
  });
}

/** Loads the registered agents. */
export function useAgents() {
  const client = useApiClient();
  return useQuery({
    queryKey: agentsKey,
    queryFn: ({ signal }) => fetchAgents(client, signal),
  });
}

/** Starts a run and puts it in the caches. */
export function useStartRun() {
  const client = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: { config_id: string; agents?: string[] }) => startRun(client, request),
    onSuccess: (run: Run) => {
      queryClient.setQueryData(runKey(run.id), run);
      void queryClient.invalidateQueries({ queryKey: runsKey, exact: true });
    },
  });
}

/** Stops a run and puts the answer in the cache. */
export function useStopRun(id: string) {
  const client = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (force: boolean) => stopRun(client, id, force),
    onSuccess: (run: Run) => {
      queryClient.setQueryData(runKey(id), run);
      void queryClient.invalidateQueries({ queryKey: runsKey, exact: true });
    },
  });
}
