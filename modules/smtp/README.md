# smtp module

The `smtp` module generates SMTP dialogues between a mail client and a mail server, for relay between servers on port 25 and for authenticated submission on port 587. Its native crate `fps-module-smtp` implements ten template functions in Rust under decision D10, so scenarios render every command, reply, and message without a WebAssembly runtime. The message bodies and the attachments come from the `content` module, which the manifest requires. Every user name, password, address, and host name is synthetic, and every address lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Six flow programs, listed below |
| `native/` | The crate `fps-module-smtp` with the MIME and DATA renderers, the name generator, the `TemplateFn` implementation `Smtp`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The capture `smtp.pcapng`, its labels `expected.yaml`, `capture.sh`, which records it, and `zeek_check.py` with its tests |

## Template functions

Text arguments are checked before any byte is written. A header value with a line break, a NUL byte, or a bad name, type, or count fails the session with the reason `extension_invalid_argument`, so no argument can add a header or a command. Calls that return addresses return bytes, and `message` takes its address parameters as bytes for that reason, which lets a scenario nest `smtp.address(...)` in it.

| Function | Parameters with defaults | Result |
|---|---|---|
| `credential` | `field`, `seed=-1` | A synthetic `user`, an address of the form `first.last@example.test`, or a `password` such as `Tst-Kamurai4821`. Every password starts with `Tst-`. A seed of zero or more fixes the value, and -1 draws from the session generator. |
| `address` | `seed`, `index=1`, `domain='example.test'` | The address that the pair `seed` and `index` names. The same pair always gives the same address, so a scenario derives the sender and the recipients of a message from one seed. |
| `recipients` | `seed`, `count`, `domain` | The addresses 1 to `count`, separated by a comma and a space, for a `To` header |
| `rcpt_to` | `seed`, `count=1`, `start=1`, `domain` | `RCPT TO:<address>` lines, each ending in CRLF, for the addresses `start` to `start + count - 1`. One line per call serves a sequential dialogue and many lines serve a pipelined batch. |
| `auth_plain` | `user`, `password`, `authzid=''` | The base64 token of an `AUTH PLAIN` initial response, `authzid NUL user NUL password` as RFC 4616 defines it |
| `base64` | `data`, `wrap=0` | Base64 text. A `wrap` of 4 or more, in multiples of 4, ends each line after that many characters. The `AUTH LOGIN` answers use it. |
| `message` | `sender`, `to`, `subject`, `body`, `cc=''`, `attachment=''`, `filename='attachment.bin'`, `content_type='application/octet-stream'`, `mailer` | A message with CRLF line breaks. The headers are `From`, `To`, `Cc`, `Subject`, `Date`, `Message-ID`, `MIME-Version`, and `X-Mailer`. Without an attachment the body is one `text/plain` part. With one, the message is `multipart/mixed` with the text part and the attachment as base64 in lines of 76 characters. The boundary, the date in 2026, and the message id come from the session generator. |
| `data` | `message` | The payload of the DATA command. It writes CRLF line breaks, doubles the leading dot of every line that starts with one, adds a final line break, and ends with the line `.`. |
| `ehlo_reply` | `domain`, `client`, `auth=''`, `pipelining=true`, `size=35882577` | The multi line 250 reply with `PIPELINING`, `SIZE`, `ETRN`, an `AUTH` line when `auth` lists mechanisms, `8BITMIME`, `ENHANCEDSTATUSCODES`, and `SMTPUTF8`. Every line but the last has a hyphen after the code. |
| `repeat_line` | `line`, `count` | `line` written `count` times, each with a CRLF, for the answers to a pipelined batch |

The `message` function wraps a text part longer than 76 characters at spaces, so a long paragraph of `content.email` stays under the line limit of RFC 5321. It sends a text part with other than ASCII characters as base64 UTF-8, and a subject with other than ASCII characters as RFC 2047 encoded words. A long list of recipients folds at the commas.

## Scenarios

Every scenario is one TCP flow. The client reads the greeting, sends `EHLO`, and reads the multi line reply up to the line that starts with `250 `. A reply of one line takes one `expect` with the pattern `\r\n`. The server reads the data of a message with the pattern `\r\n.\r\n`, which dot stuffing keeps from appearing inside a message. The text part of most messages holds a line with a single dot and a line that starts with two dots, so the scenarios exercise dot stuffing on every run. The server never checks the credentials, because a flow cannot branch on what it reads.

| Scenario | Port | Exchange |
|---|---|---|
| `relay` | 25 | No authentication. `MAIL FROM`, three `RCPT TO` lines, each answered before the next command, and `DATA` with a PNG attachment. Two recipients are in `To` and one is in `Cc`. |
| `relay_pipelined` | 25 | RFC 2920 pipelining. After `EHLO` the client writes `MAIL FROM`, every `RCPT TO` (the variable `recipients`, default 4), and `DATA` in one write. The server answers the batch in one write. The message has a PDF attachment. |
| `text_only` | 25 | One recipient and a plain text message without an attachment |
| `submission_plain` | 587 | `AUTH PLAIN` with an initial response, then two recipients and a ZIP attachment |
| `submission_login` | 587 | `AUTH LOGIN` with the 334 challenges for the user name and the password, then one recipient in `To`, one in `Cc`, and a JPEG attachment |
| `submission_batch` | 587 | `AUTH PLAIN` once, then the variable `messages` (default 3) messages on one connection, each pipelined with `recipients` (default 3) recipients and a gzip attachment, with a think time between the messages |

Each message with an attachment draws its attachment size between `attachment_min` and `attachment_max` (default 24,000 and 160,000 bytes) from the session generator, and the body from `content.email` with `body_size` bytes, so the sizes of the sessions vary. The attachment is `content.binary` of the kind that the scenario names, which writes a valid PNG, PDF, ZIP, JPEG, or gzip file.

## STARTTLS

The module has no STARTTLS scenario, because the scenario language cannot express it. A flow binds its transport once, at `connect` or `accept`, through the `transport` field, and the TLS module of TRN-04 wraps a `SessionIo` as a whole. No action upgrades the transport of a running flow after a `220 Ready to start TLS` reply. The submission scenarios therefore run on port 587 without TLS, and the server offers `AUTH` in the clear, which a lab mail server may do and which keeps the credentials visible to Zeek, tshark, and nDPI. A STARTTLS scenario needs an engine change first, an action such as `upgrade: tls` that hands the stream to the transport module in the middle of a flow, with the client and the server sides each choosing their TLS role. The implicit TLS variant on port 465 needs only the `transport` field once the MOD-04 scenarios show how a flow binds it, but Zeek then logs the connection and no message, so it would not meet the acceptance of this module.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-smtp     # validate.py on the manifest and every scenario
make test-mod-smtp     # cargo test, the Zeek check tests, then the fixture checks that the installed tools allow
sudo make e2e-mod-smtp # record and check a fresh capture in the namespace harness
```

The unit tests cover base64 against the vectors of RFC 4648, the RFC 4616 example token, line break normalization, dot stuffing, the folding of long lines and long recipient lists, the encoding of other than ASCII text and subjects, the refusal of every unsafe header value, and every function. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run both sides of every scenario over the in memory duplex for several session ids and parse both directions with a model of RFC 5321. They check the reply for every command, the multi line EHLO reply, the decoded `AUTH` credentials, the match of the `To` and `Cc` headers with the `RCPT TO` lines, the MIME structure, the decoded attachments by their magic bytes and file names, the unstuffed dot lines, the sizes of the attachments, and which commands share a write. The transcript test renders every scenario with seed 42 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and scores the capture with the realism runner. The server binds the ports 25 and 587 before its first session. The script runs 8 sessions per scenario with small attachments (2,000 to 12,000 bytes) and so records 48 sessions with 64 messages, 56 of them with an attachment. When Zeek is installed it then runs `fixtures/zeek_check.py`, which fails unless

1. `smtp.log` holds one row for every message, with the helo name, sender, recipients, `From`, `To`, subject, message id, date, and the identifiers of the files,
2. `files.log` holds one SMTP file with a name for every attachment, listed in `smtp.log`, with all bytes seen, and
3. `weird.log` holds no entry whose name starts with `smtp`.

The `realism` workflow runs `capture.sh` for this module on every change, scores the capture with nDPI, tshark, and Zeek as the [Realism](../../docs/wiki/Realism.md) page describes, and so runs the Zeek check on a fresh capture. The `mod-smtp` workflow runs the same checks on the committed capture.

The committed capture `fixtures/smtp.pcapng` records the same 8 sessions per scenario with `fixture_peer` over kernel TCP sockets on the loopback interface with `dumpcap`, because the sandbox that wrote the module had no namespace harness. It holds 64 messages in about 880 KB, and tshark 4.2.2 dissects every one as SMTP with an Internet Message Format part and reports no error. The first run of `capture.sh` in the harness replaces it, and the file `smtp.report.json` of the other modules comes with that run.
