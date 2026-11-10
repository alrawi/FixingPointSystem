// SPDX-License-Identifier: GPL-3.0-only

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, fetchAgents, releaseAgent, reserveAgent, type Agent } from "../api/client";
import { useApiClient } from "../auth/session";
import { useRunSettings } from "../runs/useRunMetrics";
import { capabilitiesKey } from "./capabilitiesQuery";
import { agentsKey } from "./runsQuery";

/** What a reservation action does to an agent. */
export type AgentAction = "reserve" | "release";

/** One reservation action on one agent. */
export interface AgentActionRequest {
  id: string;
  action: AgentAction;
}

/**
 * Loads the agents and reads them again every few seconds, because agents go offline, finish
 * runs, and get reserved by other users without anyone touching this view.
 */
export function useAgentInventory() {
  const client = useApiClient();
  const { pollMs } = useRunSettings();
  return useQuery({
    queryKey: agentsKey,
    queryFn: ({ signal }) => fetchAgents(client, signal),
    staleTime: 0,
    refetchInterval: pollMs * 5,
  });
}

/**
 * Reserves or releases one agent. A success puts the answer of the controller in the agent list.
 * A conflict (HTTP 409) or a missing agent means that the list is out of date, so the list is
 * read again at once and shows the state that the controller holds.
 */
export function useAgentAction() {
  const client = useApiClient();
  const queryClient = useQueryClient();
  return useMutation<Agent, Error, AgentActionRequest>({
    mutationFn: ({ id, action }) =>
      action === "reserve" ? reserveAgent(client, id) : releaseAgent(client, id),
    onSuccess: (agent) => {
      queryClient.setQueryData<Agent[]>(agentsKey, (agents) =>
        agents?.map((candidate) => (candidate.id === agent.id ? agent : candidate)),
      );
      void queryClient.invalidateQueries({ queryKey: capabilitiesKey });
    },
    onError: (error) => {
      if (error instanceof ApiError && (error.status === 409 || error.status === 404)) {
        void queryClient.invalidateQueries({ queryKey: agentsKey });
      }
    },
  });
}
