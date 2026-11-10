// SPDX-License-Identifier: GPL-3.0-only
//
// Form components of the configuration editor. Each field reads its value from
// the model by path, writes edits back through the model, and shows the
// problems of exactly its own path next to the input.

import { useId, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import type { Path } from "./document";
import { useConfigModel } from "./model";

type Kind = "text" | "number" | "integer";

function display(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

/**
 * Turns the text of an input into the value that goes to the YAML. A number
 * field writes a number when the text is one and the text itself otherwise, so
 * the schema check reports what the user typed instead of the form hiding it.
 */
function parseInput(text: string, kind: Kind, optional: boolean): unknown {
  if (text.trim() === "") return optional ? undefined : "";
  if (kind === "text") return text;
  const number = Number(text);
  return Number.isFinite(number) ? number : text;
}

/** The problems of one path, as a list that an input points to with aria-describedby. */
export function Issues({ path, id }: { path: Path; id?: string }) {
  const issues = useConfigModel().issuesAt(path);
  if (issues.length === 0) return null;
  return (
    <ul className="field-errors" {...(id === undefined ? {} : { id })}>
      {issues.map((issue) => (
        <li key={`${issue.source}:${issue.message}`}>
          <span className="visually-hidden">{issue.source} problem: </span>
          {issue.message}
        </li>
      ))}
    </ul>
  );
}

interface FieldProps {
  path: Path;
  label: string;
  kind?: Kind;
  /** An empty input removes the key. */
  optional?: boolean;
  hint?: string;
  /** Values that the browser offers while typing. */
  suggestions?: readonly string[];
}

/** A text or number input bound to a path of the configuration. */
export function Field({
  path,
  label,
  kind = "text",
  optional = false,
  hint,
  suggestions,
}: FieldProps) {
  const model = useConfigModel();
  const id = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const hasIssues = model.issuesAt(path).length > 0;
  const describedBy = [hint === undefined ? "" : `${id}-hint`, hasIssues ? `${id}-err` : ""]
    .filter(Boolean)
    .join(" ");
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type="text"
        inputMode={kind === "text" ? undefined : "decimal"}
        autoComplete="off"
        spellCheck={false}
        value={draft ?? display(model.get(path))}
        disabled={!model.editable}
        aria-invalid={hasIssues || undefined}
        aria-describedby={describedBy === "" ? undefined : describedBy}
        {...(suggestions === undefined ? {} : { list: `${id}-list` })}
        onChange={(event) => {
          // Keep what the user types while the field has focus, and show the value of the YAML after.
          setDraft(event.target.value);
          model.set(path, parseInput(event.target.value, kind, optional));
        }}
        onBlur={() => {
          setDraft(null);
        }}
      />
      {suggestions !== undefined && (
        <datalist id={`${id}-list`}>
          {suggestions.map((suggestion) => (
            <option key={suggestion} value={suggestion} />
          ))}
        </datalist>
      )}
      {hint !== undefined && (
        <p id={`${id}-hint`} className="hint">
          {hint}
        </p>
      )}
      <Issues path={path} id={`${id}-err`} />
    </div>
  );
}

interface SelectFieldProps {
  path: Path;
  label: string;
  options: readonly string[];
  optional?: boolean;
  hint?: string;
}

/** A choice from a fixed list, bound to a path of the configuration. */
export function SelectField({ path, label, options, optional = false, hint }: SelectFieldProps) {
  const model = useConfigModel();
  const id = useId();
  const hasIssues = model.issuesAt(path).length > 0;
  const current = display(model.get(path));
  const describedBy = [hint === undefined ? "" : `${id}-hint`, hasIssues ? `${id}-err` : ""]
    .filter(Boolean)
    .join(" ");
  // A value outside the list stays visible, so the schema problem has something to point at.
  const known = current === "" || options.includes(current);
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={current}
        disabled={!model.editable}
        aria-invalid={hasIssues || undefined}
        aria-describedby={describedBy === "" ? undefined : describedBy}
        onChange={(event) => {
          model.set(path, event.target.value === "" ? undefined : event.target.value);
        }}
      >
        {(optional || current === "") && (
          <option value="">{optional ? "not set" : "choose"}</option>
        )}
        {!known && <option value={current}>{current}</option>}
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
      {hint !== undefined && (
        <p id={`${id}-hint`} className="hint">
          {hint}
        </p>
      )}
      <Issues path={path} id={`${id}-err`} />
    </div>
  );
}

/** Shows a value that the forms do not edit, for example `params`, and points to the YAML view. */
export function YamlOnly({ path, label }: { path: Path; label: string }) {
  const value = useConfigModel().get(path);
  if (value === undefined) return null;
  return (
    <p className="hint">
      {label}: <code>{JSON.stringify(value)}</code>. Edit it in the YAML view.
    </p>
  );
}

interface ItemListProps {
  path: Path;
  /** What one item is called, in lower case, for example `port`. */
  noun: string;
  /** The item that Add appends. */
  create: (index: number) => unknown;
  children: (index: number, itemPath: Path) => ReactNode;
}

/** A list of items with add, remove, and move buttons. */
export function ItemList({ path, noun, create, children }: ItemListProps) {
  const model = useConfigModel();
  const raw = model.get(path);
  const items = Array.isArray(raw) ? (raw as unknown[]) : [];
  const capital = noun.charAt(0).toUpperCase() + noun.slice(1);
  return (
    <div className="item-list">
      <Issues path={path} />
      {items.map((_, index) => {
        const itemPath = [...path, index];
        const name = `${noun} ${String(index + 1)}`;
        return (
          // Items have no identity of their own, so the index is the key.
          <fieldset className="item" key={index}>
            <legend>{`${capital} ${String(index + 1)}`}</legend>
            <Issues path={itemPath} />
            <div className="item-fields">{children(index, itemPath)}</div>
            <div className="item-actions">
              <Button
                isDisabled={!model.editable || index === 0}
                aria-label={`Move up ${name}`}
                onPress={() => {
                  model.move(path, index, index - 1);
                }}
              >
                Move up
              </Button>
              <Button
                isDisabled={!model.editable || index === items.length - 1}
                aria-label={`Move down ${name}`}
                onPress={() => {
                  model.move(path, index, index + 1);
                }}
              >
                Move down
              </Button>
              <Button
                isDisabled={!model.editable}
                aria-label={`Remove ${name}`}
                onPress={() => {
                  model.remove(path, index);
                }}
              >
                Remove
              </Button>
            </div>
          </fieldset>
        );
      })}
      <Button
        isDisabled={!model.editable}
        onPress={() => {
          model.add(path, create(items.length));
        }}
      >
        {`Add ${noun}`}
      </Button>
    </div>
  );
}
