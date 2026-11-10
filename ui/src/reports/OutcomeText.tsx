// SPDX-License-Identifier: GPL-3.0-only

import "./reports.css";
import { criterionWord, type Outcome } from "./compare";

/** The outcome of a criterion or a realism property as a word. The class only adds color. */
export function OutcomeText({ outcome, word }: { outcome: Outcome; word?: string }) {
  return (
    <span className={`outcome-word outcome-${outcome}`}>{word ?? criterionWord(outcome)}</span>
  );
}
