// SPDX-License-Identifier: GPL-3.0-only

import { Link } from "react-router";
import type { ModuleSummary } from "../api/client";
import { modulePath } from "./moduleFormat";
import { useModules, useSignOutWhenUnauthorized } from "./modulesQuery";
import { useHeadingFocus } from "./useHeadingFocus";

/**
 * Lists the installed modules. Each module name links to its detail view, so
 * Tab moves through the modules and Enter opens one.
 */
export function ModulesPage() {
  const query = useModules();
  const unauthorized = useSignOutWhenUnauthorized(query.error);
  const heading = useHeadingFocus<HTMLHeadingElement>();

  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Modules
      </h1>
      {query.isPending && (
        <p role="status" aria-live="polite">
          Loading modules
        </p>
      )}
      {query.isError && !unauthorized && (
        <p role="alert" className="notice">
          Modules could not be loaded. {query.error.message}
        </p>
      )}
      {query.data && <ModuleList modules={query.data} />}
    </>
  );
}

function ModuleList({ modules }: { modules: ModuleSummary[] }) {
  if (modules.length === 0) {
    return (
      <p>
        No module is installed. An administrator installs signed module archives through the API.
      </p>
    );
  }
  return (
    <>
      <p>
        {modules.length === 1
          ? "One module is installed."
          : `${modules.length} modules are installed.`}
      </p>
      <table aria-label="Installed modules">
        <thead>
          <tr>
            <th scope="col">Module</th>
            <th scope="col">Version</th>
            <th scope="col">Type</th>
            <th scope="col">License</th>
            <th scope="col">Scenarios</th>
            <th scope="col">Summary</th>
          </tr>
        </thead>
        <tbody>
          {modules.map((module) => (
            <tr key={`${module.name}@${module.version}`}>
              <th scope="row">
                <Link
                  to={modulePath(module.name, module.version)}
                  aria-label={`${module.name} ${module.version}`}
                >
                  {module.name}
                </Link>
              </th>
              <td>{module.version}</td>
              <td>{module.type}</td>
              <td>{module.license}</td>
              <td>{(module.scenarios ?? []).join(", ")}</td>
              <td>{module.summary ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
