// SPDX-License-Identifier: GPL-3.0-only

import { useState } from "react";
import { Link } from "react-router";
import "../runs/runs.css";
import { ApiError, type Agent } from "../api/client";
import { formatLabels, formatLinkSpeed, holderOf } from "./agentFormat";
import { useAgentAction, useAgentInventory, type AgentAction } from "./agentsQuery";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { useHeadingFocus } from "./useHeadingFocus";

/** The result of the last reserve or release, shown in a live region. */
interface Outcome {
  ok: boolean;
  text: string;
}

const PAST: Record<AgentAction, string> = { reserve: "reserved", release: "released" };

/** Lists the agents with their capabilities, reservations, and ports, and reserves and releases them. */
export function AgentsPage() {
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const agents = useAgentInventory();
  const action = useAgentAction();
  const [outcome, setOutcome] = useState<Outcome | undefined>(undefined);
  const unauthorized = useSignOutWhenUnauthorized(agents.error ?? action.error);

  function run(agent: Agent, kind: AgentAction) {
    setOutcome(undefined);
    action.mutate(
      { id: agent.id, action: kind },
      {
        onSuccess: (updated) => {
          setOutcome({
            ok: true,
            text: `Agent ${agent.id} ${PAST[kind]}. It is now ${updated.state}.`,
          });
        },
        onError: (error) => {
          const reason = error instanceof ApiError ? error.message : "Try again.";
          // A 409 means that the controller refused because of the state it holds: another
          // holder, no control session, or a plan. Its detail names which one, and the list
          // refreshes to the current state and holder.
          const conflict = error instanceof ApiError && error.status === 409;
          setOutcome({
            ok: false,
            text: conflict
              ? `Agent ${agent.id} was not ${PAST[kind]} because of a conflict. ${reason}`
              : `Agent ${agent.id} was not ${PAST[kind]}. ${reason}`,
          });
        },
      },
    );
  }

  const list = agents.data ?? [];
  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Agents
      </h1>
      {agents.isPending && (
        <p role="status" aria-live="polite">
          Loading agents
        </p>
      )}
      {agents.isError && !unauthorized && (
        <p role="alert" className="notice">
          Agents could not be loaded. {agents.error.message}
        </p>
      )}
      <div aria-live="polite" role="status">
        {action.isPending && "Waiting for the controller."}
        {outcome?.ok === true && outcome.text}
      </div>
      {outcome?.ok === false && !unauthorized && (
        <p role="alert" className="notice">
          {outcome.text}
        </p>
      )}
      {agents.data && (
        <>
          <p className="hint">{summarize(list)}</p>
          <AgentTable agents={list} busy={action.isPending} onAction={run} />
          <PortTable agents={list} />
        </>
      )}
    </>
  );
}

/** One sentence that counts the agents by state, such as `3 agents: 1 online, 2 reserved`. */
function summarize(agents: Agent[]): string {
  if (agents.length === 0) return "No agent is registered.";
  const counts = new Map<string, number>();
  for (const agent of agents) counts.set(agent.state, (counts.get(agent.state) ?? 0) + 1);
  const parts = ["online", "reserved", "busy", "offline"]
    .filter((state) => counts.has(state))
    .map((state) => `${String(counts.get(state))} ${state}`);
  return `${String(agents.length)} ${agents.length === 1 ? "agent" : "agents"}: ${parts.join(", ")}`;
}

function AgentTable({
  agents,
  busy,
  onAction,
}: {
  agents: Agent[];
  busy: boolean;
  onAction: (agent: Agent, action: AgentAction) => void;
}) {
  if (agents.length === 0) {
    return (
      <p>
        No agent is registered. An agent appears here after it dials the controller or after an
        administrator enrolls its identity.
      </p>
    );
  }
  return (
    <section aria-labelledby="agents-heading">
      <h2 id="agents-heading">Registered agents</h2>
      <table aria-label="Agents">
        <thead>
          <tr>
            <th scope="col">Agent</th>
            <th scope="col">State</th>
            <th scope="col">Backend</th>
            <th scope="col">Cores</th>
            <th scope="col">Version</th>
            <th scope="col">Address</th>
            <th scope="col">Labels</th>
            <th scope="col">Held by</th>
            <th scope="col">Reservation</th>
          </tr>
        </thead>
        <tbody>
          {agents.map((agent) => (
            <tr key={agent.id}>
              <th scope="row">{agent.id}</th>
              <td>
                <span className={`state state-${agent.state}`}>{agent.state}</span>
              </td>
              <td>{agent.backend ?? "not reported"}</td>
              <td>{agent.cores ?? "not reported"}</td>
              <td>{agent.version ?? "not reported"}</td>
              <td>{agent.address ?? "offline"}</td>
              <td>{formatLabels(agent.labels)}</td>
              <td>
                <HolderCell agent={agent} />
              </td>
              <td>
                <ReservationCell agent={agent} busy={busy} onAction={onAction} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function ReservationCell({
  agent,
  busy,
  onAction,
}: {
  agent: Agent;
  busy: boolean;
  onAction: (agent: Agent, action: AgentAction) => void;
}) {
  if (agent.state === "online") {
    return (
      <button
        type="button"
        disabled={busy}
        onClick={() => {
          onAction(agent, "reserve");
        }}
      >
        {`Reserve ${agent.id}`}
      </button>
    );
  }
  if (agent.state === "reserved" && holderOf(agent).kind === "run") {
    // The controller would end the reservation under the run, so the view offers no release.
    return <span>Held by a run</span>;
  }
  if (agent.state === "reserved") {
    return (
      <button
        type="button"
        disabled={busy}
        onClick={() => {
          onAction(agent, "release");
        }}
      >
        {`Release ${agent.id}`}
      </button>
    );
  }
  return <span>{agent.state === "busy" ? "In use by a run" : "Not connected"}</span>;
}

/** Names the holder of the reservation, and links a run holder to the page of that run. */
function HolderCell({ agent }: { agent: Agent }) {
  const holder = holderOf(agent);
  switch (holder.kind) {
    case "none":
      return <span>none</span>;
    case "operator":
      return <span>operator</span>;
    case "run":
      return <Link to={`/runs/${encodeURIComponent(holder.id)}`}>{`Run ${holder.id}`}</Link>;
    case "unknown":
      return <span>not reported</span>;
  }
}

function PortTable({ agents }: { agents: Agent[] }) {
  const rows = agents.flatMap((agent) => (agent.ports ?? []).map((port) => ({ agent, port })));
  return (
    <section aria-labelledby="ports-heading">
      <h2 id="ports-heading">Ports</h2>
      {rows.length === 0 ? (
        <p>No agent reports a port.</p>
      ) : (
        <table aria-label="Ports">
          <thead>
            <tr>
              <th scope="col">Agent</th>
              <th scope="col">Port</th>
              <th scope="col">Driver</th>
              <th scope="col">Speed</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ agent, port }) => (
              <tr key={`${agent.id}/${port.name}`}>
                <th scope="row">{agent.id}</th>
                <td>{port.name}</td>
                <td>{port.driver ?? "not reported"}</td>
                <td>{formatLinkSpeed(port.speed_mbps)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
