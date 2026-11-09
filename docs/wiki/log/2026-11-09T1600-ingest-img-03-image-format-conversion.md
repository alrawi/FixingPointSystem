## [2026-11-09] ingest | IMG-03 Image format conversion

IMG-03 adds `image/convert.sh`, which converts the qcow2 image to raw, VHDX, a fixed VHD aligned to 1 MiB (Azure), and an OVA with a streamOptimized VMDK, an OVF 1.0 descriptor, and a SHA256 manifest. It writes `SHA256SUMS` and runs `qemu-img check` where the format supports it. `image/tests/convert-test.sh` tests it on a tiny qcow2 and runs inside `make test-img`. We updated the [Image](../Image.md) page and `image/README.md`.

### Shared page notes

- `mk/img.mk`: add `image/convert.sh` and `image/tests/convert-test.sh` to `IMG_SHELL_FILES` so that `make lint-img` checks them.
- Gate 3 (QA-10) can call `image/convert.sh` for its image formats check.
