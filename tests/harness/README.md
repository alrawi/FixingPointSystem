# Namespace harness

The harness connects a client namespace and a server namespace through a routing namespace that stands in for the device under test. End to end tests use it to run real sockets on any Linux host or CI runner without virtual machines.

```text
 <p>-client               <p>-router                 <p>-server
 eth0[.vlan] ------- cli0[.vlan]   srv0[.vlan] ------- eth0[.vlan]
 client net .2        client net .1   server net .1      server net .2
```

The router takes the first host address of each prefix, and the client and server take the second. The client and server route everything through the router, which forwards between its two arms. Every link and route lives inside the three namespaces, so the root namespace stays untouched and `down` removes everything by deleting the namespaces.

## Files

| File | Purpose |
|---|---|
| `netns.sh` | The harness with the commands `up`, `down`, `exec`, `capture`, and `info` |
| `pcapng.py` | Converts the pcap file of tcpdump to pcapng and prints a JSON summary of a pcapng file |
| `selftest.sh` | Component test that runs the harness in several configurations, needs root |
| `test_pcapng.py` | Unit tests of `pcapng.py` |
| `requirements.txt` | Python tools for the lint and unit test targets |

## Commands

Every command except `info` needs root. The script checks this and never calls `sudo` itself.

```bash
sudo tests/harness/netns.sh up
sudo tests/harness/netns.sh exec server python3 -m http.server --bind 10.202.0.2 8080 &
sudo tests/harness/netns.sh capture start out/run.pcapng client
sudo tests/harness/netns.sh exec client curl http://10.202.0.2:8080/
sudo tests/harness/netns.sh capture stop
tests/harness/netns.sh info
sudo tests/harness/netns.sh down
```

| Command | Effect |
|---|---|
| `up` | Creates the namespaces, links, addresses, routes, and the optional VLAN and NAT, and stores the configuration. A failed `up` removes what it created. A second `up` with the same name prefix is refused. |
| `down` | Stops a running capture, kills every process in the namespaces, deletes the namespaces, and removes the state files. It succeeds when the harness is already down. |
| `exec client\|router\|server CMD...` | Runs a command in one namespace. |
| `capture start FILE [client\|server\|all]` | Records on the router's client arm (the default), server arm, or both, and returns once tcpdump listens. |
| `capture stop` | Stops the capture and writes FILE as pcapng. |
| `info` | Prints the stored configuration as `KEY=VALUE` lines, for example `SERVER_ADDR=10.202.0.2`. |

A test reads addresses from `info` and does not hard code them. The namespaces have no DNS server, so a program that resolves names at start, such as `http.server`, which looks up its own address, waits for the resolver timeout before it listens. The client and server arms record Ethernet frames with their 802.1Q tags. The `all` mode records the Linux cooked v2 link type of the `any` interface, so each forwarded packet appears once per arm.

## Configuration

The environment configures `up`. Later commands need only `FPS_NETNS` and `FPS_STATE_DIR`, and `info` prints the configuration that `up` stored.

| Variable | Default | Meaning |
|---|---|---|
| `FPS_NETNS` | `fps` | Name prefix of the namespaces, `[a-z0-9]{1,12}`; several harnesses run side by side under different prefixes |
| `FPS_CLIENT_NET` | `10.201.0.0/24` | Client arm prefix, length 8 to 30 |
| `FPS_SERVER_NET` | `10.202.0.0/24` | Server arm prefix, which must not overlap the client prefix |
| `FPS_MTU` | `1500` | MTU of every harness interface, 576 to 9000 |
| `FPS_CLIENT_VLAN` | empty | 802.1Q id on the client arm, empty for untagged |
| `FPS_SERVER_VLAN` | empty | 802.1Q id on the server arm, empty for untagged |
| `FPS_NAT` | `0` | `1` masquerades the client prefix behind the router's server arm address, with nftables or else iptables |
| `FPS_OFFLOAD` | `0` | `1` keeps the veth segmentation offloads; the default turns them off with ethtool, so captures show frames no larger than the MTU |
| `FPS_STATE_DIR` | `/run/fps-netns` | Directory of the state files |

The harness turns IPv6 off inside the namespaces, so captures hold only the traffic that tests send.

## Requirements

The harness needs `ip` from iproute2 and root. Captures need `tcpdump` and Python 3.10 or newer, VLANs need the kernel module `8021q`, NAT needs `nft` or `iptables`, and wire sized frames need `ethtool`. The self test also needs `curl` and `ss`.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-qa test-qa     # shellcheck, ruff, mypy, and the pcapng unit tests
sudo make e2e-qa         # the self test, which needs root
```

The self test brings the harness up with the defaults, then with MTU 1400, NAT, and custom prefixes, and then with a VLAN on each arm. Each case serves a file from the server namespace, fetches it with curl from the client namespace, checks the peer address that the server logs, checks that the pcapng capture holds Ethernet frames no larger than the MTU allows, and compares the namespaces, links, and routes of the root namespace before `up` and after `down`. It also checks that invalid settings, a second `up`, `exec` before `up`, and `up` without root are refused. The VLAN case is skipped on a kernel without `8021q`, unless `FPS_REQUIRE_VLAN=1` is set as in CI. The `harness` job of `.github/workflows/qa.yml` runs it on `ubuntu-24.04`.
