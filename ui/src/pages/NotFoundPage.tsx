// SPDX-License-Identifier: GPL-3.0-only

import { Link } from "react-router";

/** Shown for a path that no view handles. */
export function NotFoundPage() {
  return (
    <>
      <h1>Page not found</h1>
      <p>
        No view exists at this address. <Link to="/">Show the capabilities</Link>.
      </p>
    </>
  );
}
