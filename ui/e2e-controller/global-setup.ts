// SPDX-License-Identifier: GPL-3.0-only

import { startStack } from "./stack";

/** Starts the stack once for the whole run and returns the function that ends it. */
export default async function globalSetup() {
  const stack = await startStack();
  return async () => {
    await stack.stop();
  };
}
