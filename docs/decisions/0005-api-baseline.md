# D5. API baseline

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

Automation clients need a stable REST API, and the Open Traffic Generator (OTG) models lack stateful application objects.

## Options

- Independent REST with OTG vocabulary (chosen)
- Extend OTG directly
- Fully independent API

## Decision

We publish an independent OpenAPI 3.1 document that reuses OTG names for ports, devices, flows, and metrics, and adds host_pool, profile, and load objects. A later conformance layer can expose OTG endpoints.

## Consequences

We model applications freely. snappi clients do not work until the conformance layer exists.
