## [2026-11-09] ingest | IMG-02 first boot roles and tuning

IMG-02 adds a role selector and tuning profiles to the appliance image. The seed writes `/etc/fps/role.env` through cloud-init `write_files`, with `FPS_ROLE` set to `controller`, `agent`, or `all-in-one` (the default, which keeps the behavior of IMG-01). `fps-role.service` marks, enables, and disables the units of the role at every boot, and the controller and agent units start only when their marker exists. `fps-tune.service` applies the profile of the role. The agent profile isolates CPUs (`isolcpus`, `nohz_full`, `rcu_nocbs`, and `taskset` for the agent), reserves 2 MiB hugepages, sets the sysctls of the agent (port range, `somaxconn`, file limits) and loads the loopback notrack table when `nf_conntrack` is loaded, all sized from the CPU count and the memory of the VM. The kernel options need one reboot, which the VM does by itself, and the step is idempotent and records its result in `/etc/fps/tune.applied`. The controller role has no tuning, and all-in-one gets the sysctls only. `image/boot-test.sh` takes `--role controller|agent|all-in-one|all`, seeds the role, and checks which units run and which do not, plus the tuning (`/proc/cmdline`, isolated CPUs, hugepage count, agent pinning). The new page section is [Image](Image.md#roles-and-tuning).

The sandbox has no KVM. We ran ShellCheck, `make lint-img test-img`, and the offline tests of the role parser and the tuning logic (`image/tests/role-test.sh` and `image/tests/tune-test.sh`, with a fake systemctl, update-grub, sysctl, and nft). The owner's host runs the boot test for each role.

### Shared page notes

- [Testing](Testing.md): `make test-img` now also runs two offline tests (`role-test.sh`, `tune-test.sh`), and the boot test has a `--role` option. The agent role boots with 4 CPUs and 4 GiB.
- `mk/img.mk` lists the new scripts in `IMG_SHELL_FILES`.
- `agent_auto_enroll` stays true in every role, so a controller VM accepts any agent on its management network without enrollment. CTL-27 and AGT-08 should decide how the controller role narrows this.
- IMG-04 (CI boot test) should boot `--role all-in-one` first, and the agent role there needs a runner with at least 4 vCPUs and 4 GiB, plus time for the self reboot.
