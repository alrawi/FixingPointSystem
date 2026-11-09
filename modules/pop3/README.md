# pop3 module

The `pop3` module generates POP3 dialogues between a mail client and a mail server on port 110. Its native crate `fps-module-pop3` implements thirteen template functions in Rust under decision D10, so scenarios render every greeting banner, digest, multi line response, and dot stuffed message without a WebAssembly runtime. The server side reads each command line from `last` and answers it, so the answer fits the command that the client sent, and the message payloads come from the `content` module, which the manifest requires. Every user name, password, address, and host name is synthetic, and every address lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Six flow programs, listed below |
| `native/` | The crate `fps-module-pop3` with the reply renderers in `reply.rs`, MD5 in `md5.rs`, the MIME renderer, the name generator, the `TemplateFn` implementation `Pop3`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The capture `pop3.pcapng`, its labels `expected.yaml`, and `capture.sh`, which records it |

## How APOP, multi line responses, and dot stuffing work

A flow cannot branch on what it reads and has no counters, so the module moves those jobs into functions.

* **APOP.** The server greeting is `+OK Dovecot ready. <pid.clock@host>`, and `pop3.banner(host)` draws the timestamp banner from the session generator. The client reads the greeting with `expect: { pattern: "\r\n" }`, saves it with `set: { greeting: "{{ last }}" }`, and sends `APOP user {{ pop3.apop_digest(vars.greeting, vars.secret) }}`. The function takes the text from the first `<` to the next `>` as RFC 1939 section 7 defines it, and it returns the MD5 of that banner followed by the secret in lower case hexadecimal. The unit tests check the function against the example of the RFC (banner `<1896.697170952@dbc.mtview.ca.us>`, secret `tanstaaf`, digest `c4c9334bac560ecc979e58001b3e22fb`), and MD5 against the test suite of RFC 1321. The secret is a synthetic value with the prefix `Tst-`. The server never checks the digest, because a flow cannot branch on what it reads.
* **Server answers.** After `expect: { pattern: "\r\n" }` the command line is in `last`. `pop3.list_reply(last, ...)`, `pop3.uidl_reply(last, ...)`, `pop3.retr_reply(last, message, ...)`, `pop3.top_reply(last, message, ...)`, and `pop3.dele_reply(last, ...)` parse it and write the answer. A `last` that is not the command that the function answers fails the session with the reason `extension_protocol_violation`. A message number outside the maildrop gets `-ERR There's no message n.`, as a server writes it.
* **Multi line responses.** `CAPA`, `LIST`, `UIDL`, `RETR`, and `TOP` answer with a status line and lines that end in a line with a single dot. The client reads a single line response up to `\r\n` and a multi line response up to `\r\n.\r\n`.
* **Dot stuffing.** `pop3.retr_reply` and `pop3.top_reply` normalize every line break to CRLF and add a second dot before every line that starts with a dot (RFC 1939 section 3), so a message can never end the response early. A message that ends without a line break gets one before the closing dot line. The text part of the messages of `retrieve_all` and `fetch_delete` has a line with a single dot and a line that starts with two dots, which go out as two and three dots, so the stuffing is on the wire in every capture. The client pattern `\r\n.\r\n` cannot match stuffed content, because a lone dot in the text goes out as `..`. `pop3.dot_stuff` renders any other payload in the same way.
* **Sizes and identifiers.** The server draws a mailbox number `box` once, and `pop3.stat_reply`, `pop3.list_reply`, and `pop3.uidl_reply` derive the sizes and the unique identifiers from it. `STAT` gives the sum of the sizes that `LIST` lists, `LIST n` repeats the size of the full listing, and the identifiers have the format of Dovecot, the UID (`1000 + n`) and the UID validity as two groups of eight hex digits. The size that `RETR n` announces is the exact size of the message that follows, so it does not equal the listed size, which is drawn on its own. The `+OK n octets` line of `RETR` counts the message without the stuffing.
* **Counting.** `pop3.next(value)` adds one to a number, which the loops use for message numbers.

## Template functions

Text arguments are checked before any byte is written. A header value with a line break, a NUL byte, or a bad name, type, or count fails the session with the reason `extension_invalid_argument`. A command line from the peer that the subset below does not cover fails it with `extension_protocol_violation`.

| Function | Parameters with defaults | Result |
|---|---|---|
| `credential` | `field`, `seed=-1` | A synthetic `user`, an address of the form `first.last@example.test`, or a `password` such as `Tst-Kamurai4821`. Every password starts with `Tst-`. |
| `address` | `seed`, `index=1`, `domain='example.test'` | The address that the pair `seed` and `index` names |
| `message` | `sender`, `to`, `subject`, `body`, `cc=''`, `attachment=''`, `filename`, `content_type`, `mailer` | An RFC 5322 message with CRLF line breaks and a text part. With an attachment it is `multipart/mixed` and the attachment is base64 in lines of 76 characters. The boundary, the date in 2026, and the message id come from the session generator. |
| `next` | `value`, `step=1` | The number after `value`, as text |
| `banner` | `host`, `seed=-1` | `<pid.clock@host>`. A seed of zero or more fixes it, and -1 draws from the session generator. |
| `apop_digest` | `greeting`, `secret` | The MD5 of the banner of the greeting line and the secret, as 32 lower case hex digits |
| `dot_stuff` | `data` | `data` with CRLF line breaks, stuffed dots, and the closing `.` line |
| `stat_reply` | `count`, `box` | `+OK count total` |
| `list_reply` | `command`, `count`, `box` | The answer to `LIST` (`+OK n messages:`, one line per message, the closing dot) or to `LIST n` (`+OK n size`) |
| `uidl_reply` | `command`, `count`, `box` | The answer to `UIDL` or `UIDL n` |
| `retr_reply` | `command`, `message`, `count` | `+OK size octets`, the stuffed message, and the closing dot |
| `top_reply` | `command`, `message`, `count` | `+OK`, the headers, the blank line, and the first lines of the body that `TOP n lines` asks for, stuffed and closed by the dot |
| `dele_reply` | `command`, `count` | `+OK Marked to be deleted.` |

## Scenarios

Every scenario is one TCP flow on port 110. The server greets with `+OK Dovecot ready. <pid.clock@pop3.example.test>`, answers `CAPA` with the capability list of Dovecot without `STLS`, and answers `USER` with `+OK` and `PASS` with `+OK Logged in.`. The server never checks the credentials. The scenario variable `messages` (default 6) is the size of the maildrop.

| Scenario | Exchange |
|---|---|
| `retrieve_all` | `CAPA`, `USER` and `PASS` in clear text, `STAT`, `LIST`, `UIDL`, then `fetches` (default 3) times `RETR n` with a dot stuffed MIME message with a PDF, JPEG, or ZIP attachment, a think time, and `QUIT` |
| `apop_login` | `APOP user digest` accepted, `STAT`, `LIST`, `UIDL`, `LIST 1`, `UIDL 1`, `RETR 1` with a PDF attachment, and `QUIT`. nDPI 4.2 cannot identify it, so the fixture leaves it out, see below. |
| `apop_fallback` | `APOP user digest` rejected with `-ERR [AUTH] Authentication failed.`, then `USER` and `PASS`, `STAT`, `LIST`, `RETR 1`, and `QUIT`, as fetchmail does in its automatic mode |
| `top_preview` | `USER`, `PASS`, `STAT`, `LIST`, then `fetches` times `TOP n preview_lines` (default 10), `UIDL 1`, `RETR 1`, and `QUIT` |
| `fetch_delete` | `USER`, `PASS`, `STAT`, then `fetches` times `RETR n` and `DELE n`, one more `DELE`, `RSET` to undo the marks, and `QUIT` |
| `poll_new_mail` | `USER`, `PASS`, `STAT`, `UIDL`, and `QUIT`, the short session of a client that only checks for new mail |

Each message that arrives whole draws its attachment size between `attachment_min` and `attachment_max` (default 24,000 and 160,000 bytes) from the session generator, and its text from `content.email` with `body_size` bytes, so the sizes of the sessions vary. The attachment is `content.binary`, which writes a valid PDF, JPEG, ZIP, or gzip file. `TOP` shows the start of such a message, so the preview ends inside the MIME preamble and the headers of the parts.

## STLS

The module has no STLS scenario, and the capability list has no `STLS`. A flow binds its transport once, so no action upgrades a running flow after the server answers `STLS`. ENG-20 adds that action, and a scenario can follow.

## nDPI and the first packets

The nDPI dissector of POP3 (`mail_pop.c` of nDPI 4.2) works on each payload packet. A packet that starts with `+OK` or `-ERR` raises a stage counter by one. A packet that starts with one of the commands `AUTH`, `APOP`, `USER`, `PASS`, `CAPA`, `LIST`, `STAT`, `UIDL`, `RETR`, `DELE`, or `STLS` sets one bit of a command mask, and `PASS` also saves the password. A packet that is neither makes the dissector give up when it is the twelfth payload packet or later (`QUIT`, `TOP`, and `RSET` are not in its list, and a packet in the middle of a multi line response is neither). The dissector detects POP3 on a packet that ends in CRLF when the stage plus the number of set bits reaches 3, the stage is above zero, and either a `PASS` has saved a password or the stage is above 3.

The last condition cannot hold. The stage is a bit field of two bits in the flow state (`mail_pop_stage:2` in `ndpi_typedefs.h`), so it counts 1, 2, 3, 0, 1, and so on and never exceeds 3. nDPI 4.2 therefore identifies a POP3 session only after a `PASS` command, and a session that logs in with `APOP` alone stays unknown, although the dissector never excludes it. The first capture of this module had `apop_login` in it, and `ndpiReader` from Ubuntu 24.04 (`libndpi-bin` 4.2-2.1build1, nDPI 4.2.0) classified those 8 sessions by their port only (`Confidence: Match by port`), which the realism runner counts as unknown. For that reason

* every scenario except `apop_login` logs in with `USER` and `PASS`, and nDPI decides at the `PASS` packet, which is payload packet 4, or packet 6 after the `CAPA` of `retrieve_all` and after the rejected `APOP` of `apop_fallback`. At that packet the stage is 2 or 3, and the commands before it are all in the list of the dissector or are `+OK` and `-ERR` lines.
* `apop_fallback` is the APOP scenario that the fixture records. The `-ERR [AUTH]` answer and the `USER` and `PASS` that follow it are what fetchmail does when a server does not accept APOP.
* `apop_login` stays in the manifest, because a successful APOP login is the exchange that RFC 1939 describes, and the `capture.sh` of the fixture leaves it out with `--skip apop_login`. A later nDPI that fixes the counter would let it join, and a label that accepts the guess by port would too.

`ndpiReader` 4.2.0 classifies all 40 sessions of the committed capture as POP3 with the confidence `DPI`, and the realism runner reports the ratio 1.0 with the threshold 0.99. The `realism` workflow measures the ratio of a fresh capture of `capture.sh` in CI.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-pop3     # validate.py on the manifest and every scenario
make test-mod-pop3     # cargo test, then the fixture score if ndpiReader and tshark are installed
sudo make e2e-mod-pop3 # record and score a fresh capture in the namespace harness
```

The unit tests cover MD5 against RFC 1321, the `APOP` digest against RFC 1939, the command parser, the banner, dot stuffing, `STAT`, `LIST`, `UIDL`, `RETR`, `TOP`, and `DELE` with numbers inside and outside the maildrop, and the refusal of every unsafe argument. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run both sides of every scenario over the in memory duplex for several session ids and parse both directions with a model of RFC 1939. They check the greeting banner, the command sequence of each scenario, that each command gets exactly one answer, the credentials, the shape of the digest and that it differs between sessions, the rejection and the fallback of `apop_fallback`, the sums of `STAT` and `LIST`, the identifiers, the announced size of each `RETR` against the unstuffed message, that only the closing line is a lone dot on the wire, the dot lines of the text part before and after stuffing, the number of lines that `TOP` returns, the MIME structure and the decoded attachments by their magic bytes and file names, the size range of the attachments, and the effect of the variables. The transcript test renders every scenario with seed 42 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and scores the capture with the realism runner. It runs 8 sessions per scenario, except for `apop_login`, with small attachments (2,000 to 12,000 bytes) and so records 40 sessions. The `realism` workflow runs `capture.sh` for this module on every change and scores the capture with nDPI, tshark, and Zeek as the [Realism](../../docs/wiki/Realism.md) page describes, and the `mod-pop3` workflow scores the committed capture the same way on a GitHub hosted runner.

The committed capture `fixtures/pop3.pcapng` records the same sessions with `fixture_peer` over kernel TCP sockets on the loopback interface with `dumpcap`, because the sandbox that wrote the module had no namespace harness. It holds 40 sessions (8 of each scenario except `apop_login`) and 1,153 packets in about 870 KB. tshark dissects every command and response as POP3 (352 commands, 384 `+OK` lines, which include the 40 greetings, and the 8 `-ERR` lines of `apop_fallback`) and reports no malformed packet and no warning. The module has no Zeek log check like `zeek_check.py` of the smtp module, because we could not confirm that the Zeek release of the realism job logs POP3 traffic, so the realism job only requires that Zeek parses the capture without a parser error. The first run of `capture.sh` in the harness replaces the capture, and the file `pop3.report.json` of the other modules comes with that run.
