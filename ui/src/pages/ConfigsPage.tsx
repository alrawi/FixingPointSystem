// SPDX-License-Identifier: GPL-3.0-only

import { Link } from "react-router";
import type { ConfigRef } from "../api/client";
import { useConfigs } from "./configsQuery";
import { useSignOutWhenUnauthorized } from "./modulesQuery";
import { useHeadingFocus } from "./useHeadingFocus";

function formatTime(iso: string | undefined): string {
  if (iso === undefined) return "";
  const time = new Date(iso);
  return Number.isNaN(time.getTime()) ? iso : time.toISOString().replace("T", " ").slice(0, 16);
}

/** Lists the stored configurations. Each name opens the editor on that configuration. */
export function ConfigsPage() {
  const query = useConfigs();
  const unauthorized = useSignOutWhenUnauthorized(query.error);
  const heading = useHeadingFocus<HTMLHeadingElement>();
  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Configurations
      </h1>
      <p>
        <Link to="/configs/new">New configuration</Link>
      </p>
      {query.isPending && (
        <p role="status" aria-live="polite">
          Loading configurations
        </p>
      )}
      {query.isError && !unauthorized && (
        <p role="alert" className="notice">
          Configurations could not be loaded. {query.error.message}
        </p>
      )}
      {query.data && <ConfigList configs={query.data} />}
    </>
  );
}

function ConfigList({ configs }: { configs: ConfigRef[] }) {
  if (configs.length === 0) {
    return <p>No configuration is stored yet. Create one with New configuration.</p>;
  }
  return (
    <table aria-label="Stored configurations">
      <thead>
        <tr>
          <th scope="col">Name</th>
          <th scope="col">Id</th>
          <th scope="col">Created (UTC)</th>
        </tr>
      </thead>
      <tbody>
        {configs.map((config) => (
          <tr key={config.id}>
            <th scope="row">
              <Link to={`/configs/${encodeURIComponent(config.id)}`}>{config.name}</Link>
            </th>
            <td>{config.id}</td>
            <td>{formatTime(config.created_at)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
