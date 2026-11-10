## [2026-11-10] ingest | MOD-21 http1 captures repeat a source port

The realism job `module (http1)` failed on three fresh captures with 2 to 4 parser errors on plain HTTP sessions and passed on re-run. Each failed capture held fewer plain sessions than the peers ran (116, 116, and 118 of 120) and as many parser errors as lost sessions. The cause is a client source port that the kernel picks a second time for destination port 8080, which gives two sessions one 5-tuple. The fixture peer of `http1` now gives every session its own source port, and `capture.sh` fails with the cause when the capture holds fewer sessions than ran. The evidence is `docs/handoff/MOD-21.md`.

### Shared page notes

- [Modules](Modules.md): the http1 section gained one sentence about the source ports. The same fault can hit the peers of `dns`, `ftp`, `smtp`, `imap`, and `pop3`, because they use the kernel connector with a kernel chosen source port, and a module with many sessions to one destination is most exposed. A follow up could add a per session source port to `fps-backend-kernel` (`Connector`), or let `capture.sh` of every module check the session count as the http1 script does.
- [Testing](Testing.md) and the realism runner: the session table keys sessions by 5-tuple, so a repeated tuple shows up as a lost session and not as an error of the runner. A realism check that compares the session count with the number the peers ran would name the cause for every module.
