# imap module

The `imap` module generates IMAP dialogues between a mail client and a mail server on port 143. Its native crate `fps-module-imap` implements fourteen template functions in Rust under decision D10, so scenarios render every tag, response, and literal without a WebAssembly runtime. The server side reads each command line from `last` and answers it, so the tags match the commands that the client sent, and the message literals come from the `content` module, which the manifest requires. Every user name, password, address, and host name is synthetic, and every address lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Five flow programs, listed below |
| `native/` | The crate `fps-module-imap` with the response renderers in `reply.rs`, the MIME renderer, the name generator, the `TemplateFn` implementation `Imap`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The capture `imap.pcapng`, its labels `expected.yaml`, and `capture.sh`, which records it |

## How tags, literals, and continuations work

A flow cannot branch on what it reads and has no counters, so the module moves both jobs into functions.

* **Client tags.** The scenario variable `tag` starts at `A000`. Before each command the client runs `set: { tag: "{{ imap.next(vars.tag) }}" }`, so the commands carry `A001`, `A002`, and so on, also inside a loop. `imap.next` keeps the width of the digits, and it counts plain numbers as well, which the loops over messages use for message numbers.
* **Server tags.** After `expect: { pattern: "\r\n" }` the command line is in `last`. `imap.tagged(last, ...)` copies its tag into the tagged completion, so the server answers whatever the client sent. A `last` that is not a command line fails the session with the reason `extension_protocol_violation`.
* **Literals from the server.** `imap.fetch(last, vars.msg)` parses the `FETCH` command, renders the data items in the order that the client asked for them, and writes `BODY[] {n}` followed by the `n` bytes of the message, then the closing parenthesis. The client does not know `n` in advance, so it reads up to the end of the tagged completion.
* **Literals from the client.** The client announces a literal with `imap.literal_marker(data)`, which writes `{n}`, and waits for the `+` continuation request. The server reads the command line, saves it with `set: { cmd: "{{ last }}" }`, takes the size with `imap.literal_size(last)`, answers `+ Ready for literal data`, reads exactly that many bytes with `expect: { bytes: "{{ vars.n }}" }` and the closing CRLF, and then completes the saved command with `imap.tagged(vars.cmd, ...)`. `IDLE` uses the same saved command, because the line `DONE` arrives before the tagged completion.
* **Where the client reads to.** A `pattern` expect cannot name the tag, and message data could in principle hold any text, so every long response ends in the timing suffix that Dovecot writes, ` (0.003 + 0.001 secs).`, and the client reads up to `secs).\r\n`. The login completion has no suffix, as in Dovecot, and the client reads up to ` Logged in\r\n`. The scenario tests parse both directions in full and fail when a client stops early.

## Template functions

Text arguments are checked before any byte is written. A header value with a line break, a NUL byte, or a bad name, type, or count fails the session with the reason `extension_invalid_argument`. A command line from the peer that the subset below does not cover fails it with `extension_protocol_violation`.

| Function | Parameters with defaults | Result |
|---|---|---|
| `credential` | `field`, `seed=-1` | A synthetic `user`, an address of the form `first.last@example.test`, or a `password` such as `Tst-Kamurai4821`. Every password starts with `Tst-`. |
| `address` | `seed`, `index=1`, `domain='example.test'` | The address that the pair `seed` and `index` names |
| `auth_plain` | `user`, `password`, `authzid=''` | The base64 token of an `AUTHENTICATE PLAIN` answer, `authzid NUL user NUL password` as RFC 4616 defines it. No scenario uses it, see below. |
| `message` | `sender`, `to`, `subject`, `body`, `cc=''`, `attachment=''`, `filename`, `content_type`, `mailer` | An RFC 5322 message with CRLF line breaks and a text part. With an attachment it is `multipart/mixed` and the attachment is base64 in lines of 76 characters. The boundary, the date in 2026, and the message id come from the session generator. |
| `next` | `value` (any), `step=1` | The tag or number after `value`. `A009` gives `A010`, and `6` gives `7`. |
| `literal_marker` | `data` | `{n}` for the length of `data` |
| `literal_size` | `command` | The size, as text, that the last `{n}` or `{n+}` of the command line announces |
| `tagged` | `command`, `status='OK'`, `text='Completed'`, `code=''`, `timing=false` | `TAG STATUS [code] text` and CRLF. With `timing` the text ends in a suffix such as `(0.003 + 0.001 secs).` |
| `select_data` | `exists`, `recent=0`, `unseen=0`, `uidvalidity=-1`, `read_only=false` | The untagged `FLAGS`, `PERMANENTFLAGS`, `EXISTS`, `RECENT`, `UNSEEN`, `UIDVALIDITY`, and `UIDNEXT` lines. A negative `uidvalidity` draws one. `read_only` reports an empty `PERMANENTFLAGS`, as `EXAMINE` does. |
| `fetch` | `command`, `message=''`, `exists=0` | The untagged `FETCH` responses of the command |
| `search_reply` | `exists`, `hits` | `* SEARCH` with `hits` distinct numbers from 1 to `exists` in ascending order |
| `search_hit` | `response`, `index=1` | The `index`th number of the `SEARCH` line in `response`, as text, so the client can fetch what the server found |
| `store_reply` | `command` | The untagged `FETCH` responses with the flags after a `STORE`. A `.SILENT` item gives none. |
| `expunge_reply` | `stored` | The `EXPUNGE` lines for the messages that the saved `STORE` command flagged `\Deleted`, with the numbers shifted as each message goes |

`fetch` knows the data items `UID`, `FLAGS`, `RFC822.SIZE`, `INTERNALDATE`, `BODY[]`, `BODY[HEADER]`, `BODY[HEADER.FIELDS (...)]`, `BODY[TEXT]`, `RFC822`, `RFC822.HEADER`, and `RFC822.TEXT`, each also as `BODY.PEEK[...]`, and a `UID FETCH`. The sequence set may use numbers, ranges, lists, and `*` with `exists`. A command that asks for message data names one message and needs `message`, and a listing of `UID FLAGS RFC822.SIZE` over a range needs none. A fetch that is not a peek reports `\Seen`, as RFC 3501 requires. The mailbox model is small, message `n` has UID `1000 + n`, the flags of a message depend only on its number, so a listing and a later fetch agree, and a listed `RFC822.SIZE` is drawn on its own and does not equal the size of a later fetch.

## Scenarios

Every scenario is one TCP flow on port 143. The server greets with `* OK [CAPABILITY IMAP4rev1 ...] imap.example.test Dovecot ready.`, and the capability list has neither `STARTTLS` nor `LOGINDISABLED`. The server never checks the credentials, because a flow cannot branch on what it reads. The scenario variable `messages` (default 6) is the size of the mailbox.

| Scenario | Exchange |
|---|---|
| `mailbox_sync` | `CAPABILITY`, `LOGIN "user" "password"`, `SELECT`, `FETCH 1:N (UID FLAGS RFC822.SIZE)`, then one `FETCH n (UID FLAGS BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID)])` per message with a header literal, and `LOGOUT` |
| `fetch_messages` | `CAPABILITY`, `LOGIN`, `EXAMINE` (`READ-ONLY`, no permanent flags), then `fetches` (default 3) times `FETCH n (UID FLAGS BODY.PEEK[])` with a literal that carries a MIME message with a PDF, JPEG, or ZIP attachment, a think time, and `LOGOUT` |
| `search_store` | `CAPABILITY`, `LOGIN`, `SELECT`, `SEARCH UNSEEN`, a `FETCH` of the first hit with `BODY[]`, `STORE +FLAGS (\Seen \Flagged)`, `SEARCH FROM ... SINCE ...`, `STORE +FLAGS (\Deleted)`, `EXPUNGE`, `CLOSE`, and `LOGOUT` |
| `login_append` | `LOGIN` with the password as a literal and the `+` continuation, `STATUS "Sent"`, `APPEND "Sent" (\Seen) {n}` with a MIME message as a literal after a second `+`, the `APPENDUID` code, and `LOGOUT`. The scenario skips `CAPABILITY`. |
| `idle_push` | `CAPABILITY`, `LOGIN`, `SELECT`, `IDLE` with `+ idling`, a push of `* N+1 EXISTS` and `* 1 RECENT` after 100 to 600 ms, `DONE`, the `IDLE` completion, a fetch of the new message with `BODY.PEEK[]`, and `LOGOUT` |

Each message that arrives whole draws its attachment size between `attachment_min` and `attachment_max` (default 24,000 and 160,000 bytes) from the session generator, and its text from `content.email` with `body_size` bytes, so the sizes of the sessions vary. `fetch_messages` also draws the attachment kind. The attachment is `content.binary`, which writes a valid PDF, JPEG, ZIP, or gzip file.

## STARTTLS and AUTHENTICATE

The module has no STARTTLS scenario. A flow binds its transport once, so no action upgrades a running flow after the server answers `STARTTLS`. ENG-20 adds that action, and a scenario can follow.

The module has no `AUTHENTICATE` scenario either, although `auth_plain` renders the token. nDPI 4.2 sets the protocol of any flow that carries an `AUTHENTICATE` command to IMAPS, on the reasoning that the exchange is encrypted, so such a session would count as a miss against the acceptance of at least 99% IMAP. A scenario can follow when the realism job runs a newer nDPI or the label accepts IMAPS.

## nDPI and the first packets

The nDPI dissector of IMAP (`mail_imap.c`) reads each payload packet that ends in CRLF, takes the first word after the tag as a command, and counts the commands and the `OK` responses that it knows. It detects IMAP when the count reaches 3, 5, or 7 and a `LOGIN` has named a user, or when the count reaches 7. It gives up when a packet that it cannot read arrives as the eighth payload packet or later. In all five scenarios the server completes a quoted or literal `LOGIN` with `A00n OK [CAPABILITY ...] Logged in`, and that line is the fifth payload packet, so the dissector decides well inside the first eight packets. A port of the dissector to Python that read the committed capture classified 40 of 40 sessions as IMAP. The `realism` workflow measures the real ratio in CI.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-imap     # validate.py on the manifest and every scenario
make test-mod-imap     # cargo test, then the fixture score if ndpiReader and tshark are installed
sudo make e2e-mod-imap # record and score a fresh capture in the namespace harness
```

The unit tests cover the command parser, the sequence sets, the tag counter, the literal sizes, every `FETCH` item, the folding of header fields, `STORE` and `EXPUNGE` flags and numbers, `SELECT` data, and the refusal of every unsafe argument. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run both sides of every scenario over the in memory duplex for several session ids and parse both directions with a model of RFC 3501. They check that the client tags count up from `A001`, that each command gets exactly one tagged response with its tag, the greeting, the continuation requests, the `SELECT` data, the `SEARCH` numbers against the fetch that follows, the flags after `STORE`, the `EXPUNGE` number, the literal sizes, the MIME structure and the decoded attachments by their magic bytes and file names, the size range of the attachments, and the effect of the variables. The transcript test renders every scenario with seed 42 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and scores the capture with the realism runner. It runs 8 sessions per scenario with small attachments (2,000 to 12,000 bytes) and so records 40 sessions. The `realism` workflow runs `capture.sh` for this module on every change and scores the capture with nDPI, tshark, and Zeek as the [Realism](../../docs/wiki/Realism.md) page describes, and the `mod-imap` workflow scores the committed capture the same way on a GitHub hosted runner.

The committed capture `fixtures/imap.pcapng` records the same 8 sessions per scenario with `fixture_peer` over kernel TCP sockets on the loopback interface with `dumpcap`, because the sandbox that wrote the module had no namespace harness. It holds 40 sessions in about 700 KB, and tshark 4.2.2 dissects every one as IMAP with an Internet Message Format part and reports no malformed packet and no warning. The module has no Zeek log check like the one of the smtp module, because we could not confirm that the Zeek release of the realism job logs IMAP messages, so the realism job only requires that Zeek parses the capture without a parser error. The first run of `capture.sh` in the harness replaces the capture, and the file `imap.report.json` of the other modules comes with that run.
