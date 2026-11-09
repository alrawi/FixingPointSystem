## [2026-11-09] ingest | IMG-05 Container images

IMG-05 adds container images and a compose lab under `image/containers/`. `Dockerfile.controller` builds the static controller (Go, CGO off) into an empty root file system as user 10001, and has a second target, `tools`, with the lab helpers. `Dockerfile.agent` builds the static musl agent into an empty root file system as user 10002, with the `http1` and `dns` module trees. Base images are pinned by digest. `compose.yaml` runs one controller and two agents on a lab network: the step `prep` writes the admin API token to a volume, `enroll` mints one enrollment token per agent through the admin socket and publishes the public CA certificate, and the agents enroll over HTTPS and register over mutual TLS. `lab/run-test.sh` runs a short `http1` and `dns` test with the client side in agent1 and the server side in agent2. The workflow `containers.yml` builds the images, runs the lab on a GitHub hosted runner, and uploads the logs. We updated [Image](../Image.md) with the section Containers.

### Shared page notes

- [Operations](../Operations.md): the security rules list may name `containers.yml` as a workflow that runs on hosted runners only, with a read only token, no secrets, and no registry login or push.
- [Testing](../Testing.md): the table of workflows may list `containers.yml` as the build of the container images and the compose lab test.
- `mk/img.mk`: add `image/containers/run-lab.sh`, `image/containers/static-check.sh`, and the three scripts in `image/containers/lab/` to `IMG_SHELL_FILES`, and let `test-img` call `image/containers/static-check.sh` (it needs no Docker daemon).
