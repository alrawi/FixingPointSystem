# ssh module

The `ssh` module generates SSH 2.0 sessions between a client and a server on port 22. The sessions are real. Both sides exchange identification lines and `SSH_MSG_KEXINIT`, run a `curve25519-sha256` key exchange (RFC 8731), sign the exchange hash with an `ssh-ed25519` host key that the client verifies, derive keys, and switch to `chacha20-poly1305@openssh.com`. Authentication, the channel, and every byte of the shell or the file are encrypted packets that the receiving side authenticates with their Poly1305 tag. A session whose keys differ fails on the first encrypted packet, so a session that completes is a session whose key exchange completed on both sides.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Four scenarios with one flow each |
| `native/` | The crate `fps-module-ssh` and the `fixture_peer` example |
| `tests/` | Catalog, scenario, and golden transcript tests |
| `fixtures/` | `capture.sh`, `expected.yaml`, the committed capture `ssh.pcapng`, and the Zeek check |

## The library

The module uses the RustCrypto crates `ssh-key` (host key encoding, Ed25519 signatures), `ssh-encoding`, `x25519-dalek`, `chacha20`, `poly1305`, and `sha2`. All are licensed Apache-2.0 or MIT, which GPL-3.0 accepts. A full SSH library such as `russh` brings a client and a server on an async runtime of its own, and a scenario needs the opposite, a transport that builds and reads one packet at a time when the interpreter asks for it. The packet layer, the key exchange, and the key derivation are therefore written here, in `native/src/transport.rs` and `native/src/kex.rs`, on those primitives. The unit tests cover the RFC 4251 `mpint` vectors, the round trip of the cipher, tampered tags, replayed sequence numbers, a forged signature, and a swapped host key.

## How a session runs

One value of `Ssh` serves one session and one side. It holds the identification lines, the two `KEXINIT` payloads, the ephemeral key, the keys, and the packet counters. The interpreter builds one per flow side.

The scenario language has no action that hands received bytes to a function. A side therefore reads a packet in three steps, which every receive in the scenarios follows. An `expect` with a count of `ssh.size(last)` first reads the packet header from `last` and gives the length of the packet. A second `expect` reads that many bytes. A `set` then calls `ssh.recv(last, "<kind>")`, which decrypts the packet, checks its tag and its message number, and stores the result in the state of the module, where later calls such as `ssh.ecdh_reply` read the client key. A failed check is a protocol error, and it fails the session.

## Template functions

| Function | Purpose |
|---|---|
| `banner`, `peer_banner` | The own identification line, and the check of the peer's line |
| `kexinit` | `SSH_MSG_KEXINIT` with a random cookie and the algorithm lists |
| `ecdh_init`, `ecdh_reply` | `SSH_MSG_KEX_ECDH_INIT`, and the reply with the host key, the server key, and the signature over the exchange hash. The client verifies the signature on receipt |
| `newkeys` | `SSH_MSG_NEWKEYS`, after which the module encrypts every packet |
| `size`, `recv` | The two halves of a receive, see above |
| `service_request`, `service_accept`, `auth_none`, `auth_password`, `auth_failure`, `auth_success` | The authentication messages of RFC 4252 |
| `channel_open`, `channel_confirm`, `channel_request`, `channel_success`, `exit_status`, `channel_data`, `window_adjust`, `channel_eof`, `channel_close`, `disconnect` | The channel messages of RFC 4254 |
| `chunk`, `scp_header` | A block of synthetic file data, and the `C0644` line of the scp protocol |
| `credential`, `file_name`, `command` | Synthetic user names and passwords, file names, and command lines |
| `div`, `rem`, `flag` | Integer helpers, because the template language has no arithmetic |

The manifest lists the parameters of every function.

## Scenarios

| Scenario | Shape |
|---|---|
| `interactive_shell` | Login, a pty and a shell, a message of the day, then `commands` commands. Every command is typed key by key, which gives one 36 byte packet in each direction per key with a human gap between keys, then the output arrives in pieces and the client grants the window back. The session ends with `exit` |
| `remote_command` | Login and one `exec` request, then `output_bytes` bytes of output in `chunk` sized packets and the exit status |
| `scp_upload` | `exec` of `scp -t`, the scp handshake with its zero byte acknowledgements, then `file_bytes` bytes from the client in `chunk` sized packets, with window adjustments from the server |
| `scp_download` | The same in the other direction with `scp -f` |

`commands` and `typed` set the length of an interactive session, and `file_bytes` sets the length of a file copy. The window, the packet size, and the pauses are parameters too.

## Limits

* The module offers one set of algorithms, `curve25519-sha256`, `ssh-ed25519`, and `chacha20-poly1305@openssh.com` without a separate MAC. A capture therefore shows one kind of client. `host_key_seed` varies the host key.
* The server refuses `none` authentication first, as OpenSSH does, and then accepts the password that the scenario sends without checking it. Both sides play a script, so there is no real shell and no real file.
* A session sends no re-key and no `SSH_MSG_IGNORE`, and the packet padding is the minimum, which a real client randomizes a little.
* The output of a command is synthetic text and not the output of a real command.

## Tests

```bash
cargo test -p fps-module-ssh
```

The unit tests run the transport and the handshake. `tests/scenarios.rs` runs every scenario between two peers in memory, parses the plain prefix, checks that neither the password nor the command appears on the wire, checks the size arithmetic, and flips one bit to prove that a damaged packet fails the session. `tests/transcript.rs` compares the bytes of every scenario at seed 42 with the files under `tests/testdata/`. `UPDATE_GOLDEN=1` records an intended change.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness, runs `fixture_peer` on both sides, and records `fixtures/ssh.pcapng` with 4 sessions per scenario, 16 sessions in all. It first requires one session per run in the capture, then scores it with nDPI and tshark, which must classify every session as SSH without a parser error. When Zeek is installed, `fixtures/zeek_check.py` requires one `ssh.log` row per session with the negotiated algorithms and a single host key.

The client of `fixture_peer` takes its source port from a counter, one port per session, so that no two sessions of a capture share a 5-tuple (MOD-21).

## Dispatching the module in the agent

The agent dispatches only `http1` and `dns` today. A plan with an ssh scenario needs a registry entry for `ssh` that builds one `Ssh` per flow side, as `fixture_peer` does.
