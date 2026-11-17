// SPDX-License-Identifier: GPL-3.0-only

// Values that the stack and the specs share. The tokens exist only in the
// temporary configuration of the controller that the suite starts.

/** Token of the admin scope, which enrolls agents and installs modules. */
export const ADMIN_TOKEN = "e2e-admin-0123456789";
/** Token of the run scope, which the specs sign in with. */
export const RUN_TOKEN = "e2e-run-0123456789";
/** Token of the read scope. */
export const READ_TOKEN = "e2e-read-0123456789";

/** The fake agent that the example configurations name in their port locations. */
export const AGENT_LOCAL = "local";
/** A second fake agent that no configuration uses, for the reservation tests. */
export const AGENT_SPARE = "spare";

/** Seconds after which every fake agent ends its plan on its own. */
export const FINISH_AFTER_SECONDS = 10;

/** The port of the preview server that serves the built UI. */
export const PREVIEW_PORT = Number(process.env["FPS_E2E_PORT"] ?? "4174");

/** Environment variable that carries the base URL of the controller to the specs. */
export const CONTROLLER_URL_VAR = "FPS_E2E_CONTROLLER_URL";
