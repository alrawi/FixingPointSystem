// SPDX-License-Identifier: GPL-3.0-only

import { useEffect, useRef } from "react";
import { useLocation } from "react-router";

/**
 * Moves focus to the returned element after an in app navigation, so keyboard
 * and screen reader users start on the heading of the new view instead of on
 * the body. The first page of a session keeps the browser's own focus, which
 * leaves the skip link as the first Tab stop.
 */
export function useHeadingFocus<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const location = useLocation();
  useEffect(() => {
    // React Router gives the first entry of the history the key "default".
    if (location.key !== "default") ref.current?.focus();
  }, [location.key]);
  return ref;
}
