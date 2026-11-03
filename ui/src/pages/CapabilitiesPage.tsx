// SPDX-License-Identifier: GPL-3.0-only

import { useEffect } from "react";
import { ApiError, type Agent, type ModuleSummary } from "../api/client";
import { useSession } from "../auth/session";
import { useCapabilities } from "./capabilitiesQuery";

/** Shows the engine API version, backends, agents, and installed modules of the controller. */
export function CapabilitiesPage() {
  const { signOut } = useSession();
  const query = useCapabilities();
  const unauthorized = query.error instanceof ApiError && query.error.isUnauthorized;

  useEffect(() => {
    if (unauthorized) signOut("The controller no longer accepts the token. Sign in again.");
  }, [unauthorized, signOut]);

  return (
    <>
      <h1>Capabilities</h1>
      {query.isPending && (
        <p role="status" aria-live="polite">
          Loading capabilities
        </p>
      )}
      {query.isError && !unauthorized && (
        <p role="alert" className="notice">
          Capabilities could not be loaded. {query.error.message}
        </p>
      )}
      {query.data && (
        <>
          <section aria-labelledby="cap-engine">
            <h2 id="cap-engine">Engine</h2>
            <dl className="facts">
              <dt>Engine API</dt>
              <dd>{query.data.engine_api}</dd>
              <dt>API version</dt>
              <dd>{query.data.api_version ?? "not reported"}</dd>
              <dt>Backends</dt>
              <dd>{query.data.backends.length > 0 ? query.data.backends.join(", ") : "none"}</dd>
            </dl>
          </section>
          <AgentTable agents={query.data.agents} />
          <ModuleTable modules={query.data.modules} />
        </>
      )}
    </>
  );
}

function AgentTable({ agents }: { agents: Agent[] }) {
  return (
    <section aria-labelledby="cap-agents">
      <h2 id="cap-agents">Agents</h2>
      {agents.length === 0 ? (
        <p>No agent is registered.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th scope="col">Agent</th>
              <th scope="col">State</th>
              <th scope="col">Backend</th>
              <th scope="col">Cores</th>
              <th scope="col">Ports</th>
            </tr>
          </thead>
          <tbody>
            {agents.map((agent) => (
              <tr key={agent.id}>
                <th scope="row">{agent.id}</th>
                <td>{agent.state}</td>
                <td>{agent.backend ?? ""}</td>
                <td>{agent.cores ?? ""}</td>
                <td>{(agent.ports ?? []).map((port) => port.name).join(", ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function ModuleTable({ modules }: { modules: ModuleSummary[] }) {
  return (
    <section aria-labelledby="cap-modules">
      <h2 id="cap-modules">Modules</h2>
      {modules.length === 0 ? (
        <p>No module is installed.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th scope="col">Module</th>
              <th scope="col">Version</th>
              <th scope="col">Type</th>
              <th scope="col">License</th>
              <th scope="col">Scenarios</th>
            </tr>
          </thead>
          <tbody>
            {modules.map((module) => (
              <tr key={`${module.name}@${module.version}`}>
                <th scope="row">{module.name}</th>
                <td>{module.version}</td>
                <td>{module.type}</td>
                <td>{module.license}</td>
                <td>{(module.scenarios ?? []).join(", ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
