# D7. v1 application list

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

v1 ships 15 application modules.

## Options

- Enterprise mix (chosen)
- Web heavy mix
- Security research mix

## Decision

v1 ships HTTP/1.1, HTTP/2, HTTPS with TLS 1.2 and 1.3, DNS, SMTP, IMAP, POP3, FTP, SSH, SMB2, SIP with RTP, HLS video, NTP, LDAP, and MQTT.

## Consequences

The list covers common firewall and DPI test cases. QUIC, RDP, and Kerberos wait for later releases.
