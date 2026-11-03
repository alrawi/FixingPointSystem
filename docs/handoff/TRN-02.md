# Handoff TRN-02 Kernel UDP backend

| Field | Value |
|---|---|
| Issue | #32 |
| Branch | `wp/TRN-02` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-trn-02-d41d, lease until 2026-11-03T15:51:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement `DatagramIo` over kernel UDP sockets bound to pool addresses in `engine/fps-backend-kernel`, with connected sockets for clients, bound sockets for servers, and a `DatagramOpener` for the interpreter. A loopback test exchanges 10,000 request and response pairs from 100 source addresses in 127.0.0.0/8.

## Plan

- [x] 1. Claim, branch, handoff, and pull request
- [ ] 2. Datagram socket, connector, and listener with unit tests
- [ ] 3. Interpreter opener and loopback component tests, including the acceptance test
- [ ] 4. Transport wiki section, log entry, `make lint test`, CI, and review

## Done and verified

Nothing yet.

## Remaining

Steps 2 to 4.

## Next action

Write the datagram socket module.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Session log

- 2026-11-03 claude-trn-02-d41d claimed the package.
