## [2026-11-09] ingest | AGT-10 shared capture cell and pinned enrollment check in the agent

Three changes in `engine/fps-agent`. First, `fps_agent::control::CaptureCell` is now `pub use fps_capture::CaptureCell`, and `Prepared::start_with` puts the cell into `RuntimeConfig::capture` next to the UDP hooks, so one `StartCapture` records TCP and UDP sessions. A process with clients and servers gives its server runtime an empty cell, so a connection is recorded once, as the UDP workers already did. `tests/run_control.rs` reads the capture file of the HTTP (TCP) and DNS (UDP) plan with its own pcapng reader and finds TCP packets on the HTTP port and UDP packets in it.

Second, with `--tls-ca-sha256` and no CA file the agent now verifies the enrollment endpoint inside the TLS handshake. `PinnedVerifier` finds the pinned SHA-256 among the certificates that follow the leaf, builds a root store of that certificate, and delegates to the webpki verifier with the server name, so the leaf must chain to it. A refusal ends the connection before the HTTP request. `tests/mtls.rs` has three fake endpoints (another CA, only the leaf, a leaf of another CA under the pinned CA) and asserts that each saw no request and still holds the token, and that the agent exits permanently and keeps its token file. The fake endpoint of the older tests now presents `[leaf, CA]` as the controller does since CTL-31. The agent still checks that the `ca_pem` of the answer holds the pinned certificate before it writes `ca.pem`.

Third, the TLS 1.3 race. When the agent presented a client certificate and the first `Register` call on a fresh connection fails with `UNAVAILABLE` or `UNKNOWN` and a cause that says connection closed, broken pipe, reset, or unexpected EOF, the agent dials again at once, and it exits with the permanent `TLS failure` when the second dial ends the same way. One close stays redialable, so a controller that restarts at the wrong moment does not end the agent, and a controller that is down still gives `connection refused` and the backoff. The rule and its limits are in [Agent](../Agent.md#a-close-right-after-the-handshake). `a_certificate_that_the_controller_refuses_is_a_permanent_failure` requires `Permanent` again and ran 50 times alone and 100 times under CPU load without a failure, and a new test shows that one close is redialed and two in a row are permanent.

Findings:

- A pinned fingerprint alone now needs a controller that presents the CA in the enrollment chain (CTL-31). An older controller makes the agent fail with `the endpoint presented no certificate with the pinned fingerprint`, and a CA file still works.
- The IMG-06 and e2e flows that pass `--tls-ca-sha256` need the CTL-31 controller, which the e2e script already builds.
- Not done, by design: AGT-11 (port filter) and AGT-12.

### Shared page notes

- [Engine](../Engine.md): the open point "the agent has not switched to the shared cell" is closed by this package.
- AGT-05 log entry gap 1 and AGT-08 log entry (token open to a rogue endpoint for the first exchange) are closed by this package.
- [Testing](../Testing.md): `tests/mtls.rs` now has 19 tests and `tests/run_control.rs` checks TCP and UDP packets in one capture.
