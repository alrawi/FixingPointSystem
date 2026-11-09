## [2026-11-09] ingest | IMG-04 Image boot test in CI

IMG-04 adds the workflow `image.yml`. It builds the controller and the static musl agent, builds the image with Packer and QEMU on a GitHub hosted runner, boots the all-in-one role with KVM (TCG as fallback), runs the boot checks and a short HTTP and DNS run inside the VM through the controller REST API and the image's own agent over mutual TLS, uploads the logs and reports, and writes a job summary. The new scripts are `image/tests/vm-run.sh` and `image/ci-summary.sh`, `image/boot-test.sh` gained `--run` and `FPS_TEST_ARTIFACT_DIR`, and `image/build.sh` gained `FPS_SSH_TIMEOUT`. We updated [Image](../Image.md) with the section CI boot test.

### Shared page notes

- [Operations](../Operations.md): the security rules list may name `image.yml` as a workflow that runs on hosted runners only, with a read only token and no secrets.
- [Testing](../Testing.md): the table of workflows may list `image.yml` as the CI boot test of the image.
- `mk/img.mk`: add `image/tests/vm-run.sh` and `image/ci-summary.sh` to `IMG_SHELL_FILES`.
