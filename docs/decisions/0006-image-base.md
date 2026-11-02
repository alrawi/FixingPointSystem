# D6. Image base and builder

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

The appliance ships as qcow2 and converts to VMDK, VHDX, fixed VHD, and raw images.

## Options

- Debian stable with mkosi (recommended default)
- Ubuntu LTS with Packer (chosen)
- Debian stable with Packer
- Decide after Phase 0

## Decision

We build the appliance with Packer on an Ubuntu LTS base.

## Consequences

Ubuntu hardware enablement kernels track AF_XDP and virtio-net changes closely. Images are larger than a Debian base. Spike S3 confirms the kernel version.
