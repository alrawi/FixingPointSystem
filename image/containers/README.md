# Container images and the compose lab

IMG-05 builds a controller image and an agent image for Docker and Kubernetes labs, and a compose file that runs an all in one lab. The wiki page [Image](../../docs/wiki/Image.md), section Containers, explains the design.

## Files

| File | Purpose |
|---|---|
| `Dockerfile.controller` | Target `controller`: the static Go controller on an empty root file system, user 10001. Target `tools`: the lab helpers (shell, curl, jq, the controller binary for `fps-controller token`, the module trees, and `lab/`) |
| `Dockerfile.agent` | The static musl agent on an empty root file system, user 10002, with the `http1` and `dns` module trees |
| `Dockerfile.*.dockerignore` | The build contexts, which start at the repository root |
| `controller.yaml` | Controller settings: mutual TLS, token file, `agent_auto_enroll: false` |
| `compose.yaml` | The lab: `prep`, `controller`, `enroll`, `agent1`, `agent2`, `lab-test` |
| `test-config.yaml` | The test configuration of the lab, with `@NAME@` markers that `lab/run-test.sh` replaces |
| `lab/prep.sh`, `lab/enroll.sh`, `lab/run-test.sh` | The three steps of the lab |
| `run-lab.sh` | Builds, runs the lab, checks for leaked tokens, and collects the logs |
| `static-check.sh` | Checks of the sources that need no Docker daemon: digests, users, privileges, hadolint and ShellCheck when installed |

## Run the lab

You need Docker with the compose plugin and an x86_64 host (the agent builds for `x86_64-unknown-linux-musl`, as `image/build.sh` does). From the repository root:

```bash
image/containers/run-lab.sh            # build, run, collect logs in image/containers/out
image/containers/run-lab.sh --keep     # leave the lab running afterwards
image/containers/static-check.sh       # the checks without Docker
```

`run-lab.sh` ends with `RESULT PASS ...` or `RESULT FAIL ...` and exits with 0 for a pass. The first build compiles the agent in release mode, which takes several minutes. `out/` holds `compose.log`, one log per service, `test.log`, `inspect.json`, and `results/` with the test configurations, the run summaries, the report, and the metrics.

To build one image by hand:

```bash
docker build -f image/containers/Dockerfile.controller --target controller -t fps-controller .
docker build -f image/containers/Dockerfile.agent -t fps-agent .
```

## The lab

```
                       lab network 172.30.77.0/24 (bridge)
   +------------+      +----------------+      +----------------+
   | controller |      | agent1         |      | agent2         |
   | .2         |<-----| .5   --side    |----->| .9   --side    |
   | 8443 REST  | mTLS | client         | test | server         |
   | 8444 gRPC  |      | pool .4/30     |      | pool .8/30     |
   | 8445 enroll|      +----------------+      +----------------+
   +------------+
```

The steps run in this order:

1. `prep` writes the admin API token into the volume `secrets`.
2. `controller` starts and creates its CA and the admin socket in the volume `ctl-data`.
3. `enroll` waits for the socket, copies the public CA certificate into the volume `ca`, and mints one enrollment token per agent with `fps-controller token`. Each token lands in a file of the volume of that agent.
4. `agent1` and `agent2` start. Each reads its token file, enrolls over HTTPS on port 8445 (checking the controller certificate against `/ca/ca.crt`), deletes the token file, and registers over mutual TLS on port 8444.
5. `lab-test` installs the `http1` and `dns` modules, stores a test configuration for each agent, starts the server run on agent2 and then the client run on agent1, stops the server run, and checks the verdict.

The test follows the Gate 1 suite: two runs of one configuration, one agent each, with `--side server` and `--side client`. The mix has `http1/single_fetch` on TCP port 18080 and `dns/lookup_a` on UDP port 15353. The pools are `/30` networks of one host each. The agent counts the network and the broadcast address out and numbers a pool from network plus one, so the one host of `172.30.77.4/30` is `172.30.77.5`, the address of agent1 (the same for `.8/30` and agent2). Each pool address is the address of its container, so the agents need no route and no capability. The client run passes when its verdict is `passed` (`sessions.failed_ratio <= 0.01`) and when both sides counted succeeded sessions of both applications.

## Secrets

| Secret | Where it lives | Who reads it |
|---|---|---|
| Admin API token | file `admin.token` (0600, user 10001) in the volume `secrets` | the controller (`token_file`), `enroll`, and `lab-test`, all user 10001 |
| Enrollment token of an agent | file `enroll.token` (0640, user 10001, group 10002) in the volume `a1-enroll` or `a2-enroll`, which only that agent mounts for writing | the agent, which deletes it after it enrolled |
| CA private key, admin socket | volume `ctl-data` | the controller and `enroll`. No agent mounts it |
| CA certificate | volume `ca`, public | the agents, read only |

No token is a command line argument or an environment variable, so none shows in `docker inspect` or in a process list. The compose file has no `environment` section. `lab-test` passes the API token to `curl` through a header file in a tmpfs (`-H @file`). `run-lab.sh` reads the admin token from its volume after the test and checks that it appears in no `docker inspect` output and in no log. The admin socket belongs to user 10001 and has mode 0600, so a process in another container cannot mint tokens even when it shares the volume ownership of the lab network.

The agents use mutual TLS only. `controller.yaml` has no `insecure_plain_grpc`, and `agent_auto_enroll` is false, so only an identity that `enroll` minted a token for joins the inventory. The REST API on 8443 is plain HTTP with a bearer token, as everywhere in the project, so the lab publishes no port to the host. To reach it from the host, publish the port yourself and read the token from the volume.

## Capabilities

Every container drops all Linux capabilities (`cap_drop: [ALL]`), sets `no-new-privileges`, and has a read only root file system with a tmpfs on `/tmp`. The agent needs none in this lab:

| Capability | Needed for | In this lab |
|---|---|---|
| `NET_ADMIN` | Moving a test interface into a namespace, adding addresses or an AnyIP `local` route for a pool (`fps-hosts`, [Hosts](../../docs/wiki/Hosts.md)) | Not granted. The pools are the container addresses |
| `NET_RAW` | Raw sockets and packet capture (`AF_PACKET`) | Not granted. The lab starts no capture |
| `NET_BIND_SERVICE` | Listening on ports below 1024 | Not granted. The scenarios use ports 18080 and 15353. A test on port 80 or 53 grants it or sets `net.ipv4.ip_unprivileged_port_start=0` for the container |
| `IPC_LOCK` | Locking memory, which hugepage mappings need | Not granted. The engine of this version maps no hugepages |
| `SYS_NICE` | Real time scheduling | Not granted. The workers only pin themselves to cores, which needs no capability |

A lab that uses a real test interface (a macvlan or SR-IOV device, a pool of its own prefix) grants `NET_ADMIN` to the agent, and one that captures packets grants `NET_RAW`. Grant only those, in the compose file of that lab, and keep `cap_drop: [ALL]` before the `cap_add`. The tuning of the appliance image (CPU isolation, hugepages, sysctls) is host configuration, which a container cannot change. A container takes `sysctls:` for the namespaced ones, such as `net.ipv4.ip_local_port_range`.

## Kubernetes

The images run as numeric non root users and need no capability, so they satisfy the `restricted` Pod Security Standard when the pod sets `runAsNonRoot`, `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]`, and `seccompProfile: RuntimeDefault`. The controller needs a writable volume at `/var/lib/fps`, the agent at `/var/lib/fps-agent`, and both a tmpfs at `/tmp`. Mint the enrollment token with `fps-controller token -agent-id ID -ttl 10m -admin-socket PATH` in a container of the controller pod, and hand it to the agent through a Secret mounted as a file (`--enroll-token-file`), never as a variable or an argument. Set `tls_names` in `controller.yaml` to the service names that the agents dial. This package ships no manifests.

## Limits

1. The images build for x86_64 only, like the appliance image.
2. The lab runs both agents on one Docker host and one bridge network, so it shows the control and the application path but not the throughput of a real test interface.
3. The test pools hold one host each, so a session rate far above a few hundred per second runs out of source ports.
4. The base images are pinned by digest and do not update themselves. Update the digests together with their tags.
