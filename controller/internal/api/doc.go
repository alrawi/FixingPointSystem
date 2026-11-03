// SPDX-License-Identifier: GPL-3.0-only

// Package api serves the controller's REST API from the OpenAPI contract in
// contracts/openapi/fps-api-v1.yaml.
//
// The types, the ServerInterface, and its parameter binding wrapper in
// api.gen.go come from oapi-codegen, and the operation table in routes.gen.go
// comes from the same contract with each operation's x-required-scope. Both
// files are generated, and a test fails when they differ from the contract.
//
// Server adds what the generated code leaves out. It routes the colon actions
// of the contract, such as /configs/{id}:validate, which net/http patterns
// cannot express. It checks a bearer token and its scope before any parameter
// binding, and it reports every error as an RFC 9457 problem. Unimplemented
// answers every operation with a 501 problem, and later packages embed it and
// override the operations they implement.
package api

//go:generate go run ./gen
