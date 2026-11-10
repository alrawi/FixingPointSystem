// SPDX-License-Identifier: GPL-3.0-only
//
// The state of one configuration editor. The YAML text is the only stored
// state. The reducer parses it, checks it against the schema, and keeps the
// last value that parsed, so the forms keep showing it while the YAML view
// holds a syntax error. The forms and the YAML view edit the same text.

import { createContext, useCallback, useContext, useMemo, useReducer, type ReactNode } from "react";
import type { Document } from "yaml";
import {
  addItem,
  analyze,
  editText,
  moveItem,
  pointerOf,
  readPath,
  removeItem,
  setValue,
  type ConfigIssue,
  type Path,
} from "./document";
import { checkConfig } from "./schema";

/** The text and what the checks made of it. */
export interface Draft {
  text: string;
  /** The parsed value of the last text without a syntax error. */
  value: unknown;
  /** True when `value` belongs to the current text. */
  parsed: boolean;
  syntaxIssues: ConfigIssue[];
  schemaIssues: ConfigIssue[];
}

function read(text: string, previous: unknown): Draft {
  const analysis = analyze(text);
  if (analysis.syntaxIssues.length > 0) {
    return {
      text,
      value: previous,
      parsed: false,
      syntaxIssues: analysis.syntaxIssues,
      schemaIssues: [],
    };
  }
  return {
    text,
    value: analysis.value,
    parsed: true,
    syntaxIssues: [],
    schemaIssues: checkConfig(analysis.value),
  };
}

type Action = { type: "text"; text: string } | { type: "edit"; edit: (doc: Document) => void };

function reduce(draft: Draft, action: Action): Draft {
  const text = action.type === "text" ? action.text : editText(draft.text, action.edit);
  return text === draft.text && action.type === "edit" ? draft : read(text, draft.value);
}

/** Holds the text of one configuration and the results of the browser checks. */
export function useConfigDraft(initialText: string) {
  const [draft, dispatch] = useReducer(reduce, initialText, (text) => read(text, undefined));
  /** Replaces the whole text, as typing in the YAML view does. */
  const setText = useCallback((text: string) => {
    dispatch({ type: "text", text });
  }, []);
  /** Applies an edit to the Document of the text, as the forms do. */
  const edit = useCallback((change: (doc: Document) => void) => {
    dispatch({ type: "edit", edit: change });
  }, []);
  return { draft, setText, edit };
}

/** What the form components use. */
export interface ConfigModel {
  /** False while the text has a syntax error, because the forms then cannot edit it. */
  editable: boolean;
  get: (path: Path) => unknown;
  /** Sets a value, or removes the key when the value is undefined. */
  set: (path: Path, value: unknown) => void;
  add: (path: Path, item: unknown) => void;
  remove: (path: Path, index: number) => void;
  move: (path: Path, from: number, to: number) => void;
  /** The problems of exactly this field. */
  issuesAt: (path: Path) => ConfigIssue[];
  /** Scenario names of the installed modules, as `module/scenario`, for suggestions. */
  scenarios: readonly string[];
}

const ConfigModelContext = createContext<ConfigModel | null>(null);

export interface ConfigModelProviderProps {
  draft: Draft;
  edit: (edit: (doc: Document) => void) => void;
  /** Findings of the controller for the current text. */
  controllerIssues: readonly ConfigIssue[];
  scenarios: readonly string[];
  children: ReactNode;
}

/** Gives the form components the model of one editor. */
export function ConfigModelProvider({
  draft,
  edit,
  controllerIssues,
  scenarios,
  children,
}: ConfigModelProviderProps) {
  const model = useMemo<ConfigModel>(() => {
    const issues = [...draft.syntaxIssues, ...draft.schemaIssues, ...controllerIssues];
    return {
      editable: draft.parsed,
      get: (path) => readPath(draft.value, path),
      set: (path, value) => {
        edit((doc) => {
          setValue(doc, path, value);
        });
      },
      add: (path, item) => {
        edit((doc) => {
          addItem(doc, path, item);
        });
      },
      remove: (path, index) => {
        edit((doc) => {
          removeItem(doc, path, index);
        });
      },
      move: (path, from, to) => {
        edit((doc) => {
          moveItem(doc, path, from, to);
        });
      },
      issuesAt: (path) => {
        const pointer = pointerOf(path);
        return issues.filter((issue) => issue.pointer === pointer);
      },
      scenarios,
    };
  }, [draft, edit, controllerIssues, scenarios]);
  return <ConfigModelContext.Provider value={model}>{children}</ConfigModelContext.Provider>;
}

/** The model of the enclosing editor. */
export function useConfigModel(): ConfigModel {
  const model = useContext(ConfigModelContext);
  if (model === null) throw new Error("useConfigModel needs a ConfigModelProvider.");
  return model;
}
