// SPDX-License-Identifier: GPL-3.0-only

// Package store keeps test configurations and validates them.
//
// A Repository stores each configuration as canonical JSON under an id that
// the controller assigns. SQLite implements it with the pure Go driver
// modernc.org/sqlite, so the controller builds without cgo.
//
// Handler serves the three configuration operations of the REST contract.
// POST /v1/configs accepts a JSON or YAML body, checks it against
// contracts/schemas/test-config.schema.json, and stores it. A body that breaks
// the schema gets a 422 problem whose errors name each failing field with a
// JSON Pointer. GET /v1/configs/{id} returns the stored document, and
// POST /v1/configs/{id}:validate returns a validation report that adds the
// cross references inside the document, the scenario references against the
// installed modules, the agent capacity, and a dry run of the test compiler.
//
// The module registry and the agent manager reach the validator through the
// consumer side interfaces ModuleCatalog and AgentInventory, so this package
// imports neither of them.
package store

//go:generate cp ../../../contracts/schemas/test-config.schema.json schema/test-config.schema.json
