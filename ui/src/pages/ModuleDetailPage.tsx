// SPDX-License-Identifier: GPL-3.0-only

import { Link, useParams } from "react-router";
import { ApiError, type ModuleManifest } from "../api/client";
import { closedParameters, functionSignature, parameterRows, providesText } from "./moduleFormat";
import { useModule, useSignOutWhenUnauthorized } from "./modulesQuery";
import { useHeadingFocus } from "./useHeadingFocus";

/** Shows the manifest of one module: identity, license, scenarios, parameters, and functions. */
export function ModuleDetailPage() {
  const { name = "", version = "" } = useParams();
  const query = useModule(name, version);
  const unauthorized = useSignOutWhenUnauthorized(query.error);
  const heading = useHeadingFocus<HTMLHeadingElement>();
  const missing = query.error instanceof ApiError && query.error.status === 404;

  return (
    <>
      <p>
        <Link to="/modules">All modules</Link>
      </p>
      <h1 ref={heading} tabIndex={-1}>
        Module {name} {version}
      </h1>
      {query.isPending && (
        <p role="status" aria-live="polite">
          Loading the manifest
        </p>
      )}
      {missing && (
        <p role="alert" className="notice">
          No module named {name} is installed at version {version}.
        </p>
      )}
      {query.isError && !unauthorized && !missing && (
        <p role="alert" className="notice">
          The manifest could not be loaded. {query.error.message}
        </p>
      )}
      {query.data && <Manifest manifest={query.data} />}
    </>
  );
}

function Manifest({ manifest }: { manifest: ModuleManifest }) {
  const provides = providesText(manifest.provides);
  return (
    <>
      <section aria-labelledby="module-manifest">
        <h2 id="module-manifest">Manifest</h2>
        <dl className="facts">
          <dt>Name</dt>
          <dd>{manifest.name}</dd>
          <dt>Version</dt>
          <dd>{manifest.version}</dd>
          <dt>Type</dt>
          <dd>{manifest.type}</dd>
          <dt>Summary</dt>
          <dd>{manifest.summary ?? "none"}</dd>
          <dt>Engine API</dt>
          <dd>{manifest.engine_api}</dd>
          <dt>Manifest API</dt>
          <dd>{manifest.apiVersion}</dd>
          <dt>Provides</dt>
          <dd>{provides.length > 0 ? provides.join(", ") : "none"}</dd>
          <dt>Requires</dt>
          <dd>
            {(manifest.requires ?? []).length > 0
              ? (manifest.requires ?? [])
                  .map((entry) => `${entry.module} ${entry.version}`)
                  .join(", ")
              : "none"}
          </dd>
          <dt>Assets</dt>
          <dd>
            {(manifest.assets ?? []).length > 0
              ? (manifest.assets ?? [])
                  .map((asset) => `${asset.name} from ${asset.path ?? asset.generator ?? "?"}`)
                  .join(", ")
              : "none"}
          </dd>
          <dt>Extension</dt>
          <dd>
            {manifest.extension
              ? `${manifest.extension.wasm} with hooks ${manifest.extension.hooks.join(", ")}`
              : "none"}
          </dd>
        </dl>
      </section>
      <section aria-labelledby="module-license">
        <h2 id="module-license">License and provenance</h2>
        <dl className="facts">
          <dt>License</dt>
          <dd>{manifest.license}</dd>
          <dt>Source</dt>
          <dd>{manifest.provenance.source}</dd>
          {manifest.provenance.consent !== undefined && (
            <>
              <dt>Consent</dt>
              <dd>{manifest.provenance.consent}</dd>
            </>
          )}
          {manifest.provenance.scrubbed !== undefined && (
            <>
              <dt>Scrubbed</dt>
              <dd>{manifest.provenance.scrubbed ? "yes" : "no"}</dd>
            </>
          )}
          <dt>Notes</dt>
          <dd>{manifest.provenance.notes ?? "none"}</dd>
        </dl>
      </section>
      <Scenarios files={manifest.scenarios ?? []} />
      <Parameters parameters={manifest.parameters} />
      <Functions module={manifest.name} functions={manifest.functions ?? []} />
    </>
  );
}

function Scenarios({ files }: { files: string[] }) {
  return (
    <section aria-labelledby="module-scenarios">
      <h2 id="module-scenarios">Scenarios</h2>
      {files.length === 0 ? (
        <p>The module declares no scenario.</p>
      ) : (
        <ul>
          {files.map((file) => (
            <li key={file}>
              <code>{file}</code>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Parameters({ parameters }: { parameters: ModuleManifest["parameters"] }) {
  const rows = parameterRows(parameters);
  return (
    <section aria-labelledby="module-parameters">
      <h2 id="module-parameters">Parameters</h2>
      {rows.length === 0 ? (
        <p>The module declares no parameter.</p>
      ) : (
        <>
          {closedParameters(parameters) && <p>The module accepts only the parameters below.</p>}
          <table aria-labelledby="module-parameters">
            <thead>
              <tr>
                <th scope="col">Parameter</th>
                <th scope="col">Type</th>
                <th scope="col">Default</th>
                <th scope="col">Required</th>
                <th scope="col">Constraints</th>
                <th scope="col">Description</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.name}>
                  <th scope="row">
                    <code>{row.name}</code>
                  </th>
                  <td>{row.type}</td>
                  <td>{row.defaultValue === null ? "none" : <code>{row.defaultValue}</code>}</td>
                  <td>{row.required ? "yes" : "no"}</td>
                  <td>{row.constraints.join(", ")}</td>
                  <td>{row.description ?? ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}

function Functions({
  module,
  functions,
}: {
  module: string;
  functions: NonNullable<ModuleManifest["functions"]>;
}) {
  return (
    <section aria-labelledby="module-functions">
      <h2 id="module-functions">Functions</h2>
      {functions.length === 0 ? (
        <p>The module declares no template function.</p>
      ) : (
        <table aria-labelledby="module-functions">
          <thead>
            <tr>
              <th scope="col">Function</th>
              <th scope="col">Signature</th>
              <th scope="col">Summary</th>
            </tr>
          </thead>
          <tbody>
            {functions.map((fn) => (
              <tr key={fn.name}>
                <th scope="row">
                  <code>{fn.name}</code>
                </th>
                <td>
                  <code>{functionSignature(module, fn)}</code>
                </td>
                <td>{fn.summary}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
