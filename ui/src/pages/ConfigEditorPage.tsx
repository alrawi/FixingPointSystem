// SPDX-License-Identifier: GPL-3.0-only
//
// The test configuration editor. Forms and a YAML view edit one text. The
// browser checks the text against the JSON Schema of the contract on every
// change, and the controller validates a stored copy on request. See
// config/document.ts for what a round trip preserves.

import { useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Button } from "react-aria-components";
import { Link, useParams } from "react-router";
import {
  ApiError,
  createConfig,
  validateConfig,
  type Finding,
  type ValidationReport,
} from "../api/client";
import { useApiClient, useSession } from "../auth/session";
import { canonicalYaml, NEW_CONFIG_TEXT, type ConfigIssue } from "../config/document";
import { ConfigModelProvider, useConfigDraft } from "../config/model";
import { TEST_CONFIG_SCHEMA } from "../config/schema";
import {
  CriteriaSection,
  GeneralSection,
  LoadSection,
  NetworkSection,
  ProfileSection,
  TimelineSection,
} from "../config/sections";
import { configsKey, useConfig } from "./configsQuery";
import { useModules, useSignOutWhenUnauthorized } from "./modulesQuery";
import { useHeadingFocus } from "./useHeadingFocus";

function controllerIssue(finding: Finding): ConfigIssue {
  return {
    pointer: finding.path,
    message: finding.message,
    severity: finding.severity,
    source: "controller",
  };
}

/** The copy of the configuration that the controller holds. */
interface StoredCopy {
  id: string;
  name: string;
  /** The text that the controller holds, to tell whether the editor still shows it. */
  text: string;
}

/** The answer of the controller for one text. */
interface ControllerResult {
  text: string;
  issues: ConfigIssue[];
  report: ValidationReport | null;
}

type Notice = { kind: "ok" | "error"; text: string };

function ProblemList({ issues }: { issues: ConfigIssue[] }) {
  return (
    <ul className="problems">
      {issues.map((issue) => (
        <li key={`${issue.source}:${issue.pointer}:${issue.message}`}>
          <strong>{issue.source === "yaml" ? "YAML" : issue.source}</strong>{" "}
          {issue.line === undefined
            ? issue.pointer === ""
              ? ""
              : `${issue.pointer}: `
            : `line ${String(issue.line)}, column ${String(issue.column ?? 1)}: `}
          {issue.message}
        </li>
      ))}
    </ul>
  );
}

interface ConfigEditorProps {
  /** The text to start from. */
  initialText: string;
  /** Set when the text came from a stored configuration. */
  stored?: { id: string; name: string };
}

/** The editor for one configuration, new or loaded. */
export function ConfigEditor({ initialText, stored }: ConfigEditorProps) {
  const client = useApiClient();
  const { signOut } = useSession();
  const queryClient = useQueryClient();
  const modules = useModules();
  const { draft, setText, edit } = useConfigDraft(initialText);
  const [copy, setCopy] = useState<StoredCopy | null>(
    stored === undefined ? null : { ...stored, text: initialText },
  );
  const [result, setResult] = useState<ControllerResult | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState<"save" | "validate" | null>(null);
  const heading = useHeadingFocus<HTMLHeadingElement>();

  const scenarios = useMemo(
    () =>
      (modules.data ?? []).flatMap((module) =>
        (module.scenarios ?? []).map((scenario) => `${module.name}/${scenario}`),
      ),
    [modules.data],
  );
  // Findings of the controller describe one text. After any edit they are stale.
  const controllerIssues = useMemo(
    () => (result?.text === draft.text ? result.issues : []),
    [result, draft.text],
  );
  const issues = [...draft.syntaxIssues, ...draft.schemaIssues, ...controllerIssues];
  const clean = draft.syntaxIssues.length === 0 && draft.schemaIssues.length === 0;

  async function store(text: string): Promise<StoredCopy> {
    const ref = await createConfig(client, text);
    const next = { id: ref.id, name: ref.name, text };
    setCopy(next);
    await queryClient.invalidateQueries({ queryKey: configsKey });
    return next;
  }

  function fail(error: unknown, text: string) {
    if (error instanceof ApiError && error.isUnauthorized) {
      signOut("The controller no longer accepts the token. Sign in again.");
      return;
    }
    const findings = error instanceof ApiError ? (error.problem?.errors ?? []) : [];
    setResult({ text, issues: findings.map(controllerIssue), report: null });
    setNotice({
      kind: "error",
      text: error instanceof Error ? error.message : "The request failed.",
    });
  }

  async function save() {
    const text = draft.text;
    setBusy("save");
    setNotice(null);
    try {
      const next = await store(text);
      setResult(null);
      setNotice({ kind: "ok", text: `Saved as a new configuration with the id ${next.id}.` });
    } catch (error) {
      fail(error, text);
    } finally {
      setBusy(null);
    }
  }

  async function validate() {
    const text = draft.text;
    setBusy("validate");
    setNotice(null);
    try {
      // The controller validates only stored configurations, so a changed text is stored first.
      const target = copy?.text === text ? copy : await store(text);
      const report = await validateConfig(client, target.id);
      setResult({ text, issues: report.findings.map(controllerIssue), report });
      setNotice({
        kind: report.valid ? "ok" : "error",
        text: report.valid
          ? `The controller accepts the configuration ${target.id}.`
          : `The controller found problems in the configuration ${target.id}.`,
      });
    } catch (error) {
      fail(error, text);
    } finally {
      setBusy(null);
    }
  }

  const title = stored === undefined ? "New configuration" : `Configuration ${stored.name}`;
  const status = !draft.parsed
    ? "The YAML has a syntax error. The forms are paused until it is fixed."
    : draft.schemaIssues.length > 0
      ? `The schema check found ${String(draft.schemaIssues.length)} ${draft.schemaIssues.length === 1 ? "problem" : "problems"}.`
      : "The configuration passes the schema check in the browser.";

  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        {title}
      </h1>
      <p>
        <Link to="/configs">Back to the configurations</Link>
      </p>

      <section aria-labelledby="ed-check" className="editor-check">
        <h2 id="ed-check">Checks</h2>
        <p role="status">{status}</p>
        {issues.length > 0 && <ProblemList issues={issues} />}
        <div className="editor-actions">
          <Button
            isDisabled={!clean || busy !== null}
            onPress={() => {
              void save();
            }}
          >
            Save as new
          </Button>
          <Button
            isDisabled={!clean || busy !== null}
            onPress={() => {
              void validate();
            }}
          >
            Validate on controller
          </Button>
        </div>
        <p className="hint">
          Saving always stores a new configuration. The controller validates stored configurations
          only, so validating changed text stores it first.
        </p>
        {busy !== null && (
          <p role="status">{busy === "save" ? "Saving the configuration" : "Validating"}</p>
        )}
        {notice !== null && (
          <p role={notice.kind === "error" ? "alert" : "status"} className={notice.kind}>
            {notice.text}
            {copy !== null && notice.kind === "ok" && (
              <>
                {" "}
                <Link to={`/configs/${encodeURIComponent(copy.id)}`}>Open the stored copy</Link>
              </>
            )}
          </p>
        )}
        {result?.text === draft.text && result.report?.capacity !== undefined && (
          <p>
            Agents required {String(result.report.capacity.agents_required ?? "unknown")}, agents
            available {String(result.report.capacity.agents_available ?? "unknown")}.
          </p>
        )}
      </section>

      <div className="editor-grid">
        <div className="editor-forms">
          <ConfigModelProvider
            draft={draft}
            edit={edit}
            controllerIssues={controllerIssues}
            scenarios={scenarios}
          >
            <fieldset disabled={!draft.parsed} className="editor-fieldset">
              <legend className="visually-hidden">Configuration forms</legend>
              <GeneralSection />
              <NetworkSection />
              <ProfileSection />
              <LoadSection />
              <TimelineSection />
              <CriteriaSection />
            </fieldset>
          </ConfigModelProvider>
        </div>
        <div className="editor-yaml">
          <h2 id="yaml-heading">YAML</h2>
          <textarea
            aria-labelledby="yaml-heading"
            className="yaml"
            spellCheck={false}
            wrap="off"
            rows={36}
            value={draft.text}
            aria-invalid={!clean || undefined}
            onChange={(event) => {
              setText(event.target.value);
            }}
          />
        </div>
      </div>
    </>
  );
}

/** The route `/configs/new`. */
export function NewConfigPage() {
  return <ConfigEditor initialText={NEW_CONFIG_TEXT} />;
}

/** The route `/configs/:id`, which loads a stored configuration. */
export function StoredConfigPage() {
  const { id = "" } = useParams();
  const query = useConfig(id);
  const unauthorized = useSignOutWhenUnauthorized(query.error);
  const text = useMemo(
    () => (query.data === undefined ? null : canonicalYaml(query.data, TEST_CONFIG_SCHEMA)),
    [query.data],
  );
  const heading = useHeadingFocus<HTMLHeadingElement>();
  if (text !== null && query.data !== undefined) {
    return (
      <ConfigEditor key={id} initialText={text} stored={{ id, name: query.data.metadata.name }} />
    );
  }
  return (
    <>
      <h1 ref={heading} tabIndex={-1}>
        Configuration
      </h1>
      {query.isPending && (
        <p role="status" aria-live="polite">
          Loading the configuration
        </p>
      )}
      {query.isError && !unauthorized && (
        <p role="alert" className="notice">
          The configuration could not be loaded. {query.error.message}{" "}
          <Link to="/configs">Back to the configurations</Link>
        </p>
      )}
    </>
  );
}
