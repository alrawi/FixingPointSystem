## [2026-11-06] ingest | IMG-01 qcow2 base image

IMG-01 adds `image/`, which builds an Ubuntu 24.04 LTS qcow2 appliance with Packer's QEMU builder from the official cloud image. The image holds the controller and the agent with systemd units, cloud-init NoCloud defaults, and an isolated management interface `mgmt0`. The controller listens on the management address only. The nftables table `inet fps_isolation` drops forwarded packets into and out of `mgmt0` and drops connections to ports 22, 8443, and 8444 on other interfaces, and sysctl keeps `ip_forward` off. `image/build.sh` and `image/boot-test.sh` give a copy and paste build and a boot test that checks `GET /v1/capabilities` on the management interface and that forwarding between the management and test interfaces drops. We documented the design in the new page [Image](Image.md).

The sandbox has no KVM, Packer, or `ip`. We ran ShellCheck, `nft -c`, and the static checks locally, and the owner's host runs the build and the boot test.

### Shared page notes

- Home and the sidebar: link the new page [Image](Image.md) under the component pages.
- [Testing](Testing.md): IMG has `make lint-img`, `make test-img`, and `make e2e-img` (the namespace test of the rules). The boot test stays out of `make` because it needs KVM and a built image, and IMG-04 runs it in CI.
- `mk/img.mk` is new, and it follows the one file per workstream rule of the root Makefile.
- The module trees `modules/http1` and `modules/dns` ship in the image unfiltered. The plan digest of a module covers its tree, so a later package must decide whether the agent loads modules from the image or from the controller registry.
