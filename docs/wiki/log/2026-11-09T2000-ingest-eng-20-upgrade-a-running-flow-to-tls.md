## [2026-11-09] ingest | ENG-20 Upgrade a running flow to TLS

ENG-20 adds the flow action `tls_upgrade`, which wraps the transport of a running session in TLS in place, so STARTTLS (SMTP, IMAP, POP3, FTP, LDAP) is expressible. `fps-flow` compiles it to `Op::UpgradeTls` with a profile name, an optional role (the default is the role of the side), and an optional server name. `fps-interp` calls a `TlsUpgrader` seam, which the runner sets with `Session::set_tls_upgrader`, over a swappable transport. A program without the action runs over its stream unchanged. `fps-runtime` adds `TlsProfiles`, a registry of named `TlsConnector` and `TlsAcceptor` values of the TRN-04 module, and `RuntimeConfig::tls` hands it to every session. The capture tap of ENG-19 stays below the TLS layer, so a capture holds the encrypted records, and the `NssKeyLog` that the profiles share receives the secrets, so the capture decrypts. A failed handshake is `tls_failure`, and a missing profile, a missing upgrader, bytes buffered before the handshake, or a UDP flow is `internal`.

The loopback test `engine/fps-runtime/tests/tls_upgrade.rs` runs an inline SMTP scenario (greeting, EHLO, STARTTLS, `220`, handshake, EHLO, `MAIL FROM`, QUIT) between two runtimes with a capture. It checks that the first half is plaintext on the wire, that the mail transaction appears nowhere on the wire, and that the exported key log decrypts the second half of both directions with a decrypter built on `ring` that is independent of the TLS module. A key log with flipped digits decrypts nothing. We described the action, the seam, and the tests in [Engine](../Engine.md#tls-upgrade-of-a-running-session).

### Shared page notes

- Contract change needed (a CON package, because `contracts/` is out of the paths of ENG-20): the scenario schema allows no other action keyword, so `tls_upgrade` fails the schema and the Python validator. The engine side parses it already. Add to `$defs/action/properties` of `contracts/schemas/scenario.schema.json`:

  ```json
  "tls_upgrade": {
    "description": "TCP flows only. Upgrades the transport of the running session to TLS in place, as STARTTLS does. The client role starts the handshake and the server role accepts it. Every later action of the side runs over TLS.",
    "type": "object",
    "additionalProperties": false,
    "required": ["profile"],
    "properties": {
      "profile": { "description": "Name of a TLS profile of the run.", "type": "string", "minLength": 1 },
      "role": { "description": "The part in the handshake. Defaults to client on the client side and server on the server side.", "enum": ["client", "server"] },
      "server_name": { "description": "Name that a client handshake verifies and sends as SNI. The engine chooses when it is missing.", "type": "string", "minLength": 1 }
    }
  }
  ```

  In the action table of `contracts/README.md`, next to `spawn`: "`tls_upgrade` | Wrap the transport of the running session in TLS in place. The side's role starts or accepts the handshake, and every later action runs over TLS. TCP flows only." In the semantic rules of `tools/contracts/validate.py` and in `check_action` of `fps-flow`, a `tls_upgrade` in a UDP flow is an error (a new `LoadError` kind, for example `TlsUpgradeOnUdp`, with the same code in both). `fps-flow` accepts the action in every mode today, and the runtime fails a UDP session at the action with reason `internal`. Add a changelog line to `contracts/CHANGELOG.md`.
- Plan: a follow up for MOD-07 (or a later MOD package) can add the scenario `starttls_submission` to `modules/smtp` once the contract change merges. The test scenario of ENG-20 is a template for it, with the greeting, EHLO, STARTTLS, `220`, `tls_upgrade`, EHLO, and the mail transaction. The IMAP, POP3, FTP, and LDAP modules can use the action as well.
- `engine/fps-agent`: the agent does not build `TlsProfiles` yet, so a plan whose scenario holds `tls_upgrade` fails every session with `internal` until an agent package creates the profiles of the run (a `RunAuthority`, one connector and one acceptor per profile name, and an `NssKeyLog` that the capture download can ship) and sets `RuntimeConfig::tls`. The plan or the test configuration needs a place to name the profiles, which is a contract question for that package.
- Metrics: the runtime does not record the `tls.*` metrics of the handshake yet. `TlsSession::report()` has the values, and the interpreter would have to return it.
- [Testing](../Testing.md): the `test-eng` row names the crates under `engine/`, and `fps-runtime` now runs `tests/tls_upgrade.rs`, which needs no external tool.
- The root `Cargo.lock` gained `fps-module-tls` and `ring` in the dependency list of `fps-runtime`, both already locked. `ring` is a dev dependency of the new test.
