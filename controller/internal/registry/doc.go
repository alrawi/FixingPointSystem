// SPDX-License-Identifier: GPL-3.0-only

// Package registry keeps the controller's local module registry and serves the
// /v1/modules operations of the REST contract.
//
// A registry is a directory with one index file and one tree per installed
// module.
//
//	<root>/index.json                   the installed modules, their summaries and digests
//	<root>/modules/<name>/<version>/    the module tree, with manifest.yaml at its root
//	<root>/staging/                     archives that an install is still checking
//
// An install unpacks a tar archive (optionally gzip compressed) into the
// staging directory, validates manifest.yaml against
// contracts/schemas/module-manifest.schema.json, checks the SPDX license
// expression against a GPL-3.0 compatible allow list, checks that every file
// the manifest names exists, computes the module tree digest that
// contracts/README.md defines for plans, and then moves the tree into place
// and rewrites the index. The index is written to a temporary file and renamed,
// so a reader never sees a partial index.
//
// API serves listModules, installModule, getModule, and deleteModule and embeds
// api.Unimplemented for every other operation. Lookup is the read only view
// that the configuration validator uses to resolve module references.
package registry
