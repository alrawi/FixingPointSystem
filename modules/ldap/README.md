# ldap module

The `ldap` module generates LDAPv3 sessions (RFC 4511) between a client and a directory server on port 389. The messages are real BER. The client binds, searches, and unbinds, and the server answers from a fixture directory in LDIF, so the result of a bind and the entries of a search come from the data and are not canned. The directory is synthetic: `example.test` with 16 accounts, five department groups, and a root DSE.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `directory.ldif` | The fixture directory, 25 entries, which the server uses when `directory` is empty |
| `scenarios/*.yaml` | Four scenarios with one flow each |
| `native/` | The crate `fps-module-ldap` and the `fixture_peer` example |
| `tests/` | Catalog, scenario, golden transcript, and tshark tests |
| `fixtures/` | `capture.sh`, `expected.yaml`, the committed capture `ldap.pcapng`, and its report |

## The library

The crate has no dependency besides `fps-flow` and `rand`. The BER codec is written here, because LDAP needs a small subset of it (the definite length forms of one, two, and three octets, integers, booleans, octet strings, and constructed values), and a general ASN.1 crate would bring a schema compiler that a scenario does not use.

| Module | Contents |
|---|---|
| `ber.rs` | The writer and the reader of the BER subset, with the 4 octet head that a receive reads first |
| `filter.rs` | Search filters: the string form of RFC 4515, the BER form of RFC 4511, and the match against an entry. Extensible match is rejected |
| `message.rs` | The messages that the module speaks: bind request and response, search request, entry, and done, and unbind |
| `directory.rs` | The LDIF reader (RFC 2849, with folding, comments, and the same rules for duplicates as a server), the bind rules of RFC 4513, and the search with scope, attribute selection, and size limit |

The server never returns `userPassword`. A bind with a name and no password gets `unwillingToPerform` (53), a wrong password gets `invalidCredentials` (49), a version other than 3 gets `protocolError` (2), and SASL gets `authMethodNotSupported` (7).

## How a session runs

One value of `Ldap` serves one session and one side. It holds the message that `recv` decoded and the search that `matches` ran.

The scenario language has no action that hands received bytes to a function, so a receive takes three steps. An `expect` of 4 octets reads the head of the BER sequence, a second `expect` with the count `ldap.size(last)` reads the rest of the message, and a `set` calls `ldap.recv(last, "<kind>")`, which decodes the message, checks its kind, and keeps it. A flow cannot branch on received data, so the client counts the entries that it expects with `ldap.count` on its own copy of the directory, and the server sends each of them with `ldap.search_next`.

A nested call has the type `bytes`, which a `text` parameter does not accept, so a scenario stores a filter in a variable with a `set` action before it passes the filter on.

## Template functions

| Function | Purpose |
|---|---|
| `size`, `recv`, `field` | The two halves of a receive, and a field of the message that `recv` kept |
| `bind_request`, `bind_response`, `search_request`, `search_entry`, `search_done`, `unbind_request` | One encoded message each |
| `filter_eq`, `account`, `count` | An equality filter with the escaping of RFC 4515, an account of the directory picked by a number, and the number of entries that a search finds |
| `bind_answer`, `matches`, `search_next`, `search_end` | The server side: the answer to the bind that `recv` kept, the search that `recv` kept, the next entry of that search, and its done message |

The manifest lists the parameters of every function.

## Scenarios

| Scenario | Shape |
|---|---|
| `bind_search` | An anonymous bind to test the connection, a bind as an account of the directory, a subtree search for the entry of that account, and an unbind |
| `failed_bind` | An anonymous bind, a bind with a wrong password (49), the bind with the right one, a search for the groups that name the account as member, and an unbind |
| `directory_browse` | An anonymous bind, a one level listing of the people, a base search for one person, and an unbind |
| `rootdse_probe` | An anonymous bind, a search of the root DSE, a search of the naming context, and an unbind |

Every scenario starts with an anonymous bind. nDPI 4.2.0 labels a session MQTT when its first bind carries a name, with or without a password (measured with bind requests of 20 to 64 octets), and LDAP when the first message is the 14 octet anonymous bind. The sessions that follow the anonymous bind stay LDAP. Real clients open with an anonymous bind or a root DSE read often enough that this is no distortion.

## Limits

* The server supports simple bind and the search, entry, and done messages. It has no add, modify, delete, compare, extended, or abandon operations, no controls, and no referrals, and it rejects extensible match filters.
* Messages are limited to 65535 content octets, and a search entry that exceeds this fails the session.
* The directory is plain text in the LDIF form without base64 values.
* There is no LDAPS and no StartTLS. The `tls` module can carry them later.
