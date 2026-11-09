# ftp module

The `ftp` module generates FTP sessions between a client and a server. The control connection on port 21 carries the login, `SYST`, `FEAT`, `PWD`, `CWD`, `TYPE`, `SIZE`, `ALLO`, `PASV`, `EPSV`, `LIST`, `RETR`, `STOR`, and `QUIT` with the replies of vsftpd 3.0, including the multi line `FEAT` reply. Every `LIST`, `RETR`, and `STOR` moves its bytes over a separate passive mode data connection, which a spawned flow opens. Its native crate `fps-module-ftp` implements eleven template functions in Rust under decision D10. The bytes of the transferred files come from the `content` module, which the manifest requires. Every user name, password, file name, and listing entry is synthetic.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Five scenarios, each with a `control` flow and one or more spawned data flows |
| `native/` | The crate `fps-module-ftp` with the passive mode replies and parsers, the name and listing generator, the `TemplateFn` implementation `Ftp`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario as a scenario instance over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The capture `ftp.pcapng`, its labels `expected.yaml`, `capture.sh`, which records it, the session structure check `structure_check.py`, and the Zeek check `zeek_check.py`, each with its tests |

## How a data connection starts

A passive transfer has two connections that depend on each other. The server picks a port, opens it, and tells the client the number in the reply to `PASV` (`227 Entering Passive Mode (192,0,2,10,195,203).`) or `EPSV` (`229 Entering Extended Passive Mode (|||50123|).`). The client sends the transfer command and dials that port. The scenarios express this with the `spawn` action, which scenario instances run (ENG-07).

1. The server draws the port with `rand_int(data_port_min, data_port_max)`, binds it to the variable `data_port` with `set`, runs `spawn: { flow: data }`, waits 1 to 3 ms, and sends the reply with `ftp.pasv_reply(vars.server_ip, vars.data_port)` or `ftp.epsv_reply(vars.data_port)`. The spawned flow starts the server side at once, and `accept` opens the listener before the client learns the port. A real server listens before it announces the port as well, and a client that dials too early would meet a refused connection, which would add a session to the capture.
2. The client reads the reply into `last`, takes the port with `ftp.pasv_port(last)` or `ftp.epsv_port(last)`, and binds it with `set: { data_port: ... }`. It sends `LIST`, `RETR`, or `STOR` and runs `spawn: { flow: data }`. The spawned client side reads every `set` of its parent before the `spawn`, so `connect: { port: "{{ vars.data_port }}" }` dials the port that the reply announced.
3. The data flow moves the bytes and closes. After the transfer the server sends `226` on the control connection. The control flow does not wait for the data flow, because the scenario language has no action that waits for a spawned flow, so the server waits a uniform 10 to 40 ms before the `226` reply, a time in which the data of the small and middle sized files has arrived on a lab network. A larger file on a slow link could still have bytes in flight when the `226` reply arrives, which is harmless to a parser.
4. The k-th `spawn` of a flow on the client meets the k-th `spawn` of the same flow on the server. `batch_download` spawns its `data` flow once per file, and `mixed_session` uses a different flow for each kind of transfer.

The data flow needs the size of the transfer on both sides, because `expect bytes` reads an exact number, and a flow cannot read until the peer closes. The sizes travel the way that the real protocol sends them.

| Transfer | Who knows the size | How the other side learns it |
|---|---|---|
| `RETR` | The server, which draws a content seed and a size in its `SIZE` handler and computes the length of the file with `ftp.length(content.binary(..., seed=vars.file_seed))` | `213 <size>` as the reply to `SIZE`, and the client reads it with `ftp.number(last)`. The `150` reply repeats it in the usual `(<size> bytes)` form. |
| `STOR` | The client, with the same seeded file | `ALLO <size>` before `STOR`, which the server answers with `202 ALLO command ignored.` and reads with `ftp.number(last)` |
| `LIST` | Both, because the variable `list_entries` gives the number of lines | The client reads that many lines, each ending in CRLF |

A fixed `seed` makes `content.binary` return the same bytes and so the same length on every call, which is why the control flow can measure the file and the data flow can send it.

## Template functions

Text arguments are checked before any byte is written. A reply that a function parses must have the right code and the right shape, a host must be a dotted IPv4 address, and a count or a port outside its range fails the session with the reason `extension_invalid_argument`.

| Function | Parameters with defaults | Result |
|---|---|---|
| `credential` | `field`, `seed=-1` | A synthetic `user`, two invented words joined by a dot, or a `password` such as `Tst-Kamurai4821`. Every password starts with `Tst-`. A seed of zero or more fixes the value, and -1 draws from the session generator. |
| `file_name` | `seed`, `ext='bin'` | A name such as `bratos_zeilan-202603.pdf`. The same seed and extension always give the same name. |
| `listing` | `seed`, `count` | `count` lines of an `ls -l` listing with a mode, a link count, `ftp` as owner and group, a size, a date, and a name, each ending in CRLF. About one entry in six is a directory. The same seed and count always give the same text. |
| `length` | `data` | The number of bytes of `data` as a decimal number |
| `number` | `line` | The last run of decimal digits of a command line or a reply. It reads the size of an `ALLO` command or a `213` reply. |
| `arg` | `line` | The text after the first space of a command line, without the line break, for the file name in the `150` reply |
| `pasv_reply` | `host`, `port` | `227 Entering Passive Mode (h1,h2,h3,h4,p1,p2).` with CRLF, where `p1 * 256 + p2` is the port |
| `pasv_port` | `reply` | The port that a 227 reply announces. It accepts the other wordings of servers, because it reads only the six numbers between the parentheses. |
| `epsv_reply` | `port` | `229 Entering Extended Passive Mode (\|\|\|port\|).` with CRLF. The reply names no address, so the client uses the address of the control connection (RFC 2428). |
| `epsv_port` | `reply` | The port that a 229 reply announces |
| `feat_reply` | `utf8=true` | The multi line `211` reply with the features of vsftpd, ` EPRT`, ` EPSV`, ` MDTM`, ` PASV`, ` REST STREAM`, ` SIZE`, ` TVFS`, and ` UTF8`, between `211-Features:` and `211 End` |

## Scenarios

Every scenario is a scenario instance with one `control` flow and the spawned data flows. The client reads the greeting, logs in with `USER` and `PASS` with synthetic credentials, and reads one reply for each command. The server never checks the credentials, because a flow cannot branch on what it reads. Each reply is a single line, and `FEAT` ends at the line `211 End`.

| Scenario | Control commands | Data connections |
|---|---|---|
| `list_pasv` | `USER`, `PASS`, `SYST`, `FEAT`, `PWD`, `CWD`, `TYPE A`, `PASV`, `LIST`, `QUIT` | One, a listing of `list_entries` lines (default 24) |
| `retr_epsv` | `USER`, `PASS`, `SYST`, `PWD`, `CWD`, `TYPE I`, `SIZE`, `EPSV`, `RETR`, `QUIT` | One, a PDF of 40,000 to 400,000 bytes |
| `stor_pasv` | `USER`, `PASS`, `SYST`, `FEAT`, `CWD`, `TYPE I`, `ALLO`, `PASV`, `STOR`, `QUIT` | One, a ZIP file that the client sends and closes first |
| `mixed_session` | Login, `SYST`, `FEAT`, `PWD`, `CWD`, then `EPSV` and `LIST`, `SIZE`, `PASV`, and `RETR`, and `ALLO`, `EPSV`, and `STOR`, with a think time between the transfers, and `QUIT` | Three, a listing, a gzip download, and a PNG upload, in three flows `data_list`, `data_retr`, and `data_stor` |
| `batch_download` | Login, `SYST`, `PWD`, `CWD`, `TYPE I`, then `SIZE`, `PASV`, and `RETR` in a loop of `files` iterations (default 4), `QUIT` | `files`, each a ZIP file with its own size and its own port |

The file sizes lie between `file_min` and `file_max` (default 40,000 and 400,000 bytes). `content.binary` writes at least the size that it is asked for, so a file can exceed `file_max` by a small header, and `SIZE` reports the real length. The listings follow the `ls -l` format with sizes up to 90 MB, which are only text and carry no data.

## Active mode and TLS

The module has no active mode. A `PORT` or `EPRT` command makes the server open the data connection from port 20 to the client, so the server side of the spawned flow would have to `connect` and the client side would have to `accept` on the port that the command names. The scenario language can express that, because a flow chooses its roles per side, but `PORT` needs the address of the client in the command, and a scenario has no reference to the address of its own host. Passive mode is what clients use today, so MOD-10 leaves active mode out, and a later package can add scenarios once a variable carries the client address.

The module has no `AUTH TLS` scenario for the reason that the smtp module gives for STARTTLS. The scenario language cannot upgrade the transport of a running flow.

## The address in the 227 reply

A `227` reply carries an IPv4 address, and a flow has no reference to the address of its own server. The variable `server_ip` (default `192.0.2.10`, a documentation address) names it. `fixture_peer` sets it to the address that it listens on, so a client and Zeek find the data connection at the announced address, and a run on the controller must set it to the address of the server pool for the same effect. The `229` reply of `EPSV` names no address and needs no variable. The `connect` of a data flow names no host, so the engine picks the server from its pool, which also keeps the data connection on the server that holds the control connection in a one server setup.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-ftp      # validate.py on the manifest and every scenario
make test-mod-ftp      # cargo test, the fixture check tests, the structure check, then the checks that the installed tools allow
sudo make e2e-mod-ftp  # record and check a fresh capture in the namespace harness
```

The unit tests cover the passive mode replies and their parsers (round trips, the other wordings, and every malformed reply), the `number` and `arg` readers, the listing format, and every argument check. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default.

The scenario tests run every scenario as a scenario instance, both sides of every flow, over an in memory network that records the bytes of each connection. They parse the control connection with a model of RFC 959, RFC 2428, and RFC 3659 and require the reply code of every command, the multi line `FEAT` reply, and the announced host and port. They then check the session structure for several seeds and session ids. A scenario has exactly one control connection to port 21 and exactly one data connection for each `LIST`, `RETR`, and `STOR`, opened to the announced port with the announced passive mode. The bytes of each data connection must match the transfer, which means the listing lines, the length that `SIZE` or `ALLO` announced, and the magic bytes of the PDF, ZIP, gzip, or PNG file. Further tests change `files`, `list_entries`, the file sizes, and the port range, and check that sessions draw their own sizes and ports.

The transcript test renders every scenario with seed 42 and first session id 1, with `file_min` 300, `file_max` 400, `list_entries` 6, and `files` 2, and compares the bytes of the control connection and of each data connection with `tests/testdata/<scenario>.seed42.control.c2s.bin` and `.s2c.bin`, and `<scenario>.seed42.data<n>.<c2s|s2c>.bin`. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and checks the capture. The server binds port 21 before its first session and binds each passive port when its spawned flow starts. The script runs 8 sessions per scenario with small files (2,000 to 12,000 bytes) and 16 listing lines. That records 40 control connections and 80 data connections, 16 of them listings, 48 downloads, and 16 uploads.

The script runs three checks on the capture.

1. The realism runner scores the 40 control connections with nDPI and tshark and requires at least 99% classified as FTP_CONTROL. The data connections have no rule in `expected.yaml`, because they carry a listing or a file without a protocol of their own.
2. `structure_check.py` is the acceptance check for the session structure. RLM-08, the session structure check of the realism runner, does not exist yet, so the script reads the capture with the CAP-01 extractor (`python3 -m capture sessions --payloads`) and counts connections itself. It requires one control connection for each FTP session, exactly one other TCP session of the same addresses that starts while the control connection is open for each `LIST`, `NLST`, `MLSD`, `RETR`, `STOR`, `APPE`, and `STOU`, a data connection to the port of the n-th `227` or `229` reply, and a handshake, bytes, and a FIN of both sides in every session. A refused or repeated data connection fails it. When RLM-08 exists, its scenario graph can replace this script.
3. When Zeek is installed, `zeek_check.py` requires that `ftp.log` holds a `PASV` or `EPSV` row for every transfer with a passive data channel and the announced port, and a `RETR` or `STOR` row for every file transfer with a user, a file name, a reply code below 400, and the file identifier. It requires that `files.log` holds an `FTP_DATA` file with bytes and no lost bytes for every transfer, listings included, that the data ports of those files are exactly the announced ports, which shows that Zeek tied each data connection to its control connection, and that `weird.log` holds no entry that starts with `ftp`. Zeek 9.0.1 logs the data channel only in the rows of `PASV` and `EPSV` and logs no row for `LIST`, which is why the check counts listings through `files.log`.

The `realism` workflow runs `capture.sh` for this module on every change. The `mod-ftp` workflow runs the same checks on the committed capture, with the structure check and the Zeek check as separate steps.

The committed capture `fixtures/ftp.pcapng` records the same 8 sessions per scenario with `fixture_peer` over kernel TCP sockets on the loopback interface with `dumpcap`, with the passive ports 20000 to 20999 so that they stay below the ephemeral port range of the client on the same host. It holds 120 sessions in about 730 KB. `structure_check.py` passes on it, and tshark 4.2.2 dissects every data connection as `ftp-data` with the setup frame of the `227` or `229` reply that announced it. The first run of `capture.sh` in the harness replaces it, and the file `ftp.report.json` of the other modules comes with that run.

## Dispatching the module in the agent

The agent dispatches native functions by module name, and `engine/fps-agent/src/modules.rs` does not list `ftp` yet. A plan with an ftp scenario fails every call until the agent routes `ftp` calls to `fps_module_ftp::Ftp` and `content` calls to `fps_module_content::Content`, and `engine/fps-agent/Cargo.toml` depends on both crates. The module does not edit the agent.
