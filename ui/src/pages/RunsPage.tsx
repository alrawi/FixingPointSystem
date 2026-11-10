// SPDX-License-Identifier: GPL-3.0-only

import { useState, type SyntheticEvent } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import "../runs/runs.css";
import { ApiError, type Agent, type ConfigRef, type Run } from "../api/client";
import { formatTimestamp } from "../runs/format";
import { useConfigs } from "./configsQuery";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { useAgents, useRuns, useStartRun } from "./runsQuery";
import { StateBadge } from "./StateBadge";
import { useHeadingFocus } from "./useHeadingFocus";

/** Starts runs and lists them. Each run links to its live view. */
export function RunsPage() {
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const runs = useRuns();
  const configs = useConfigs();
  const unauthorized = useSignOutWhenUnauthorized(runs.error);
  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Runs
      </h1>
      <StartRunForm configs={configs.data} configsFailed={configs.isError} />
      <h2>Recent runs</h2>
      <p>
        <Link to="/compare">Compare two runs</Link>
      </p>
      {runs.isPending && (
        <p role="status" aria-live="polite">
          Loading runs
        </p>
      )}
      {runs.isError && !unauthorized && (
        <p role="alert" className="notice">
          Runs could not be loaded. {runs.error.message}
        </p>
      )}
      {runs.data && <RunList runs={runs.data} configs={configs.data ?? []} />}
    </>
  );
}

function StartRunForm({
  configs,
  configsFailed,
}: {
  configs: ConfigRef[] | undefined;
  configsFailed: boolean;
}) {
  const navigate = useNavigate();
  const [search] = useSearchParams();
  const agents = useAgents();
  const start = useStartRun();
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  const [selected, setSelected] = useState<string[]>([]);
  // A link such as /runs?config=cfg_x preselects a configuration that exists.
  const fromLink = search.get("config") ?? "";
  const config =
    chosen ?? (configs?.some((candidate) => candidate.id === fromLink) === true ? fromLink : "");

  function submit(event: SyntheticEvent) {
    event.preventDefault();
    if (config === "") return;
    start.mutate(
      { config_id: config, ...(selected.length > 0 ? { agents: selected } : {}) },
      {
        onSuccess: (run) => {
          void navigate(`/runs/${encodeURIComponent(run.id)}`);
        },
      },
    );
  }

  const agentList = agents.data ?? [];
  return (
    <section aria-labelledby="start-heading">
      <h2 id="start-heading">Start a run</h2>
      <form onSubmit={submit} className="run-form">
        <div className="field">
          <label htmlFor="run-config">Configuration</label>
          <select
            id="run-config"
            value={config}
            onChange={(event) => {
              setChosen(event.target.value);
            }}
            disabled={configs === undefined}
          >
            <option value="">Choose a stored configuration</option>
            {(configs ?? []).map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {`${candidate.name} (${candidate.id})`}
              </option>
            ))}
          </select>
          {configsFailed && (
            <p role="alert" className="notice">
              The configurations could not be loaded.
            </p>
          )}
          {configs?.length === 0 && (
            <p className="hint">
              No configuration is stored yet. <Link to="/configs/new">Create one</Link> first.
            </p>
          )}
        </div>
        <fieldset className="editor-fieldset">
          <legend>Agents</legend>
          <p className="hint">
            Leave every agent unchecked and the controller picks the reserved agents.
          </p>
          {agents.isError && (
            <p role="alert" className="notice">
              The agents could not be loaded. {agents.error.message}
            </p>
          )}
          <AgentChoices agents={agentList} selected={selected} onChange={setSelected} />
        </fieldset>
        <div>
          <button type="submit" disabled={config === "" || start.isPending}>
            Start run
          </button>
        </div>
        {start.isPending && (
          <p role="status" aria-live="polite">
            Starting the run
          </p>
        )}
        {start.isError && (
          <p role="alert" className="notice">
            The run did not start.{" "}
            {start.error instanceof ApiError ? start.error.message : "Try again."}
          </p>
        )}
      </form>
    </section>
  );
}

function AgentChoices({
  agents,
  selected,
  onChange,
}: {
  agents: Agent[];
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  if (agents.length === 0) return <p className="hint">No agent is registered.</p>;
  return (
    <ul className="plain">
      {agents.map((agent) => {
        const usable = agent.state === "online" || agent.state === "reserved";
        return (
          <li key={agent.id}>
            <label>
              <input
                type="checkbox"
                checked={selected.includes(agent.id)}
                disabled={!usable}
                onChange={(event) => {
                  onChange(
                    event.target.checked
                      ? [...selected, agent.id]
                      : selected.filter((id) => id !== agent.id),
                  );
                }}
              />{" "}
              {agent.id} ({agent.state})
            </label>
          </li>
        );
      })}
    </ul>
  );
}

function RunList({ runs, configs }: { runs: Run[]; configs: ConfigRef[] }) {
  if (runs.length === 0) return <p>No run exists yet. Start one with the form above.</p>;
  const nameOf = (id: string) => configs.find((config) => config.id === id)?.name ?? id;
  return (
    <table aria-label="Runs">
      <thead>
        <tr>
          <th scope="col">Run</th>
          <th scope="col">Configuration</th>
          <th scope="col">State</th>
          <th scope="col">Created (UTC)</th>
          <th scope="col">Report</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((run) => (
          <tr key={run.id}>
            <th scope="row">
              <Link to={`/runs/${encodeURIComponent(run.id)}`}>{run.id}</Link>
            </th>
            <td>{nameOf(run.config_id)}</td>
            <td>
              <StateBadge state={run.state} />
            </td>
            <td>{formatTimestamp(run.created_at)}</td>
            <td>
              <Link to={`/runs/${encodeURIComponent(run.id)}/report`}>{`Report of ${run.id}`}</Link>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
