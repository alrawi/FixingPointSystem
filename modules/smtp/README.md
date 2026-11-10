# smtp module

The `smtp` module generates SMTP dialogues between a mail client and a mail server, for relay between servers on port 25 and for authenticated submission on port 587, with and without STARTTLS. Its native crate `fps-module-smtp` implements ten template functions in Rust under decision D10, so scenarios render every command, reply, and message without a WebAssembly runtime. The message bodies and the attachments come from the `content` module, which the manifest requires. Every user name, password, address, and host name is synthetic, and every address lies under the reserved `.test` domain.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Seven flow programs, listed below |
| `native/` | The crate `fps-module-smtp` with the MIME and DATA renderers, the name generator, the `TemplateFn` implementation `Smtp`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture, with the TLS profiles of `tests/common/tls.rs` for the STARTTLS scenario |
| `tests/` | Catalog, interpreter, STARTTLS, fixture, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The capture `smtp.pcapng` with its embedded TLS keys, its labels `expected.yaml`, `capture.sh`, which records it, `zeek_check.py`, and `decrypted_check.py`, each with its tests |

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
| `ehlo_reply` | `domain`, `client`, `auth=''`, `pipelining=true`, `size=35882577`, `starttls=false` | The multi line 250 reply with `PIPELINING`, `SIZE`, `ETRN`, a `STARTTLS` line when `starttls` is true, an `AUTH` line when `auth` lists mechanisms, `8BITMIME`, `ENHANCEDSTATUSCODES`, and `SMTPUTF8`. Every line but the last has a hyphen after the code. |
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
| `starttls_submission` | 587 | The greeting, `EHLO` and a reply that offers `STARTTLS` and no `AUTH`, `STARTTLS` and `220 2.0.0 Ready to start TLS`, then the TLS handshake on both sides. Inside TLS the client sends `EHLO` again, the server offers `AUTH PLAIN LOGIN`, and the client sends `AUTH PLAIN`, two recipients, and a message with a PDF attachment. |

Each message with an attachment draws its attachment size between `attachment_min` and `attachment_max` (default 24,000 and 160,000 bytes) from the session generator, and the body from `content.email` with `body_size` bytes, so the sizes of the sessions vary. The attachment is `content.binary` of the kind that the scenario names, which writes a valid PNG, PDF, ZIP, JPEG, or gzip file.

## STARTTLS

The scenario `starttls_submission` (MOD-20) uses the action `tls_upgrade` of ENG-20. After the `220` reply both sides run `tls_upgrade`. The client uses the profile `firefox-139` with the server name `submit.example.test`, and the server uses `server-tls13`, so the session runs on TLS 1.3 and the `EHLO`, the `AUTH`, and the message are encrypted. A client does not trust what a server offered in the clear, so it sends `EHLO` again inside TLS, and the server then offers `AUTH` and no `STARTTLS`. The server name of `tls_upgrade` is a literal, as the action requires, and equals the default of the variable `server_name`, so a run that changes one must change the other too. The function `ehlo_reply` gained the parameter `starttls`, which puts the `STARTTLS` line after `ETRN`, as Postfix does.

Two gaps remain. The browser profiles offer the ALPN ids `h2` and `http/1.1`, which a mail client never sends, and the server profiles select none. The agent needs a mail client profile without ALPN and a mail server profile for the same reason, and until it has them a plan that follows this scenario shows the browser ClientHello to a mail server. The profiles of `tests/common/tls.rs` are a copy of the registry of the `http1` tests, which the agent will replace with its own builder (AGT-14). The implicit TLS variant on port 465 needs only the `transport` field, but Zeek then logs the connection and no message, and the module has no scenario for it.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-smtp     # validate.py on the manifest and every scenario
make test-mod-smtp     # cargo test, the Zeek check tests, then the fixture checks that the installed tools allow
python3 -m unittest discover -s modules/smtp/fixtures   # the tests of both check scripts, which need no tool
sudo make e2e-mod-smtp # record and check a fresh capture in the namespace harness
```

The STARTTLS tests (`tests/starttls.rs`) run the scenario over the in memory duplex with the TLS profiles and check on the dialogue and on the wire that the `EHLO` reply offers `STARTTLS` and no `AUTH`, that the `220` reply is the last plaintext, that the dialogue inside TLS is complete, with the decoded PDF, that the handshake is TLS 1.3 with the SNI of the scenario and no ALPN id, that the credentials and the message never appear in the records, that the key log names the hello on the wire and holds the secrets of TLS 1.3, and that the ClientHello has the JA4 of `firefox-139`. The test `tests/fixture.rs` runs `fixtures/decrypted_check.py` on the committed capture. The unit tests cover base64 against the vectors of RFC 4648, the RFC 4616 example token, line break normalization, dot stuffing, the folding of long lines and long recipient lists, the encoding of other than ASCII text and subjects, the refusal of every unsafe header value, and every function. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run both sides of every scenario over the in memory duplex for several session ids and parse both directions with a model of RFC 5321. They check the reply for every command, the multi line EHLO reply, the decoded `AUTH` credentials, the match of the `To` and `Cc` headers with the `RCPT TO` lines, the MIME structure, the decoded attachments by their magic bytes and file names, the unstuffed dot lines, the sizes of the attachments, and which commands share a write. The transcript test renders every scenario with seed 42 and compares both directions with `tests/testdata/<scenario>.seed42.c2s.bin` and `.s2c.bin`. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and scores the capture with the realism runner. The server binds the ports 25, 587, and 5587 before its first session. The script runs 8 sessions per scenario with small attachments (2,000 to 12,000 bytes) and so records 56 sessions, 48 in the clear with 64 messages, 56 of them with an attachment, and 8 with STARTTLS. When Zeek is installed it then runs `fixtures/zeek_check.py`, which fails unless

1. `smtp.log` holds one row for every message of the plain sessions, with the helo name, sender, recipients, `From`, `To`, subject, message id, date, and the identifiers of the files,
2. `files.log` holds one SMTP file with a name for every attachment, listed in `smtp.log`, with all bytes seen, and
3. `weird.log` holds no entry whose name starts with `smtp`.

The check leaves out a row of a STARTTLS session that has no sender, because Zeek stops reading such a session at `STARTTLS`. The `realism` workflow runs `capture.sh` for this module on every change, scores the capture with nDPI, tshark, and Zeek as the [Realism](../../docs/wiki/Realism.md) page describes, and so runs the Zeek check on a fresh capture. The `mod-smtp` workflow scores the committed capture.

**The STARTTLS sessions use port 5587 in the fixture.** The labels tell sessions apart by their server port, and port 587 carries the plain submission sessions, so `capture.sh` runs `starttls_submission` on 5587 with `fixture_peer --port starttls_submission=5587`. The scenario itself keeps 587.

**Keys.** The client peer writes the secrets of every handshake to a key log in the NSS format, and `capture.sh` injects the log into the capture with `editcap --inject-secrets tls,keys.log`. The capture so carries its own keys as a decryption secrets block. `expected.yaml` lists the profile `firefox-139` with the expected JA4 `t13d1011h2_61a7ad8aa9b6_f674693fb0dc` and requires one decrypted session at least. The encryption check of the realism runner decrypts all 8 STARTTLS sessions with the embedded keys and finds that the JA4 of every ClientHello matches. `FPS_FIXTURE_KEYLOG` keeps the key log as a file too.

**What nDPI says.** nDPI 4.2.0 labels the 8 STARTTLS sessions `SMTP`, as it does the plain ones. It settles on SMTP at the greeting and the first `EHLO`, reads the SNI of the ClientHello, and does not revise the protocol, so it never gives `SMTPS` or `TLS`. The labels accept `SMTP` and `SMTPS` for them, so a later nDPI that revises the label still passes. The label differs from the one of the imap module, where nDPI says `IMAPS`.

**What Zeek can do.** Zeek has no way to decrypt TLS, because it takes no key log. On the capture it sees the plaintext start of a STARTTLS session, may log it as a row with `tls` true and no sender, and then sees TLS records, so it logs no message of these sessions. `fixtures/decrypted_check.py` therefore uses a decrypted view, as the MOD-04 check does for HTTPS. It decrypts the capture with tshark and the embedded keys, takes the plaintext of each session (what both sides sent before the `ClientHello`, then what they sent inside TLS, read with `follow,tls`), and writes it as a capture of plain TCP sessions on port 587 with the addresses of the original sessions. The view leaves out the `STARTTLS` command and its `220` reply, because a reader stops at them, and it is synthetic, built from the output of tshark. The script then checks three things.

1. tshark parses the SMTP dialogue inside every TLS session. Each session shows `EHLO` again, `AUTH`, `MAIL`, `RCPT`, `DATA`, one Internet Message Format message, and `QUIT`, with the replies 235, 354, and 221.
2. nDPI classifies the 8 sessions of the view as `SMTP` (ratio 1.0, no parser error).
3. With `--zeek`, Zeek reads the view, and the checks of `zeek_check.py` hold on it: `smtp.log` has one message per session with every header field and the file of the attachment, `files.log` has the attachment with all its bytes, and `weird.log` has no `smtp` entry. This part could not run in the sandbox that wrote the module, because Zeek is not installed there. It ran in the `realism` workflow with Zeek 9.0.1 on the capture of that run: `smtp.log` held 8 messages, `files.log` 16 files, and no check failed. The step for the `mod-smtp` workflow is in the log entry of MOD-20.

So the acceptance that Zeek logs the messages inside TLS holds for the view and not for the capture itself.

The committed capture `fixtures/smtp.pcapng` is the output of `capture.sh` in the namespace harness (nDPI 4.2.0 and tshark 4.2.2) with the 56 sessions and about 1.1 MB. tshark dissects every plain session as SMTP with an Internet Message Format part and reports no error, and the realism check passes for the conformance, the parser errors, and the encryption. The file `smtp.report.json` of the other modules comes with the next run of `capture.sh`.
