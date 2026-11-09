# SPDX-License-Identifier: GPL-3.0-only
#
# IMG appliance image. The boot test needs KVM and a built image, so only the
# static checks and the root only namespace test join the root targets.

IMG_SHELL_FILES := image/build.sh image/boot-test.sh image/tests/static-check.sh \
	image/tests/vm-checks.sh image/tests/isolation-netns.sh \
	image/tests/role-test.sh image/tests/tune-test.sh \
	image/scripts/provision.sh image/scripts/cleanup.sh \
	image/files/usr/libexec/fps/fps-mgmt-net image/files/usr/libexec/fps/fps-mgmt-addr \
	image/files/usr/libexec/fps/fps-init-token image/files/usr/libexec/fps/fps-agent-start \
	image/files/usr/libexec/fps/fps-role image/files/usr/libexec/fps/fps-tune \
	image/files/usr/libexec/fps/fps-enroll-token image/tests/enroll-token-test.sh \
	image/convert.sh image/tests/convert-test.sh \
	image/tests/vm-run.sh image/ci-summary.sh

# Syntax of every script, and ShellCheck and Packer when they are installed.
LINT_TARGETS += lint-img
lint-img:
	for f in $(IMG_SHELL_FILES); do bash -n "$$f"; done
	if command -v shellcheck >/dev/null; then shellcheck -x $(IMG_SHELL_FILES); \
	else echo "lint-img: shellcheck is not installed, skipped"; fi
	if command -v packer >/dev/null; then packer fmt -check -diff image/fps.pkr.hcl; \
	else echo "lint-img: packer is not installed, skipped"; fi

# Static checks of the sources, without root, QEMU, or network.
TEST_TARGETS += test-img
test-img:
	image/tests/static-check.sh

# The namespace test proves that the rules block forwarding. It needs root, ip,
# nft, ping, curl, and python3. The boot test, image/boot-test.sh, stays out of
# make because it needs KVM and a built image (see image/README.md).
E2E_TARGETS += e2e-img
e2e-img:
	image/tests/isolation-netns.sh
