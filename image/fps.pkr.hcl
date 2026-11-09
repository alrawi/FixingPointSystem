# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the FixingPointSystem appliance as a qcow2 image (IMG-01).
#
# The build boots the official Ubuntu 24.04 LTS cloud image under QEMU, copies in
# the controller, the agent, their systemd units, the cloud-init defaults, and the
# nftables rules, then shuts the VM down and keeps the disk. image/build.sh makes
# the binaries and calls `packer init` and `packer build` with the variables below.

packer {
  required_version = ">= 1.9.0"
  required_plugins {
    qemu = {
      version = ">= 1.1.0"
      source  = "github.com/hashicorp/qemu"
    }
  }
}

variable "base_image_url" {
  type        = string
  description = "Official Ubuntu 24.04 LTS (noble) cloud image. Use a dated release URL to pin the base."
  default     = "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-amd64.img"
}

variable "base_image_checksum" {
  type        = string
  description = "Checksum of the base image, or a file: reference to the SHA256SUMS of the release."
  default     = "file:https://cloud-images.ubuntu.com/releases/noble/release/SHA256SUMS"
}

variable "controller_binary" {
  type        = string
  description = "Path of the fps-controller binary for linux amd64."
}

variable "agent_binary" {
  type        = string
  description = "Path of the fps-agent binary for linux amd64."
}

variable "modules_dir" {
  type        = string
  description = "Directory that holds the module trees the agent loads, one directory per module."
}

variable "output_directory" {
  type        = string
  description = "Directory that receives fps-base.qcow2. Packer replaces it when build.sh passes -force."
  default     = "output"
}

variable "ssh_password" {
  type        = string
  description = "Password of the throwaway packer user of the build VM. build.sh makes a random one."
  sensitive   = true
}

variable "accelerator" {
  type        = string
  description = "kvm for a real build, or tcg for software emulation, which is slow."
  default     = "kvm"
}

variable "disk_size" {
  type        = string
  description = "Virtual size of the root disk. The cloud image grows its root file system into it at first boot."
  default     = "8G"
}

variable "memory" {
  type        = number
  description = "Memory of the build VM in MiB."
  default     = 2048
}

variable "cpus" {
  type        = number
  description = "vCPUs of the build VM."
  default     = 2
}

variable "ssh_timeout" {
  type        = string
  description = "How long Packer waits for ssh in the build VM. Software emulation (tcg) boots slowly and needs more than the default."
  default     = "20m"
}

variable "headless" {
  type        = bool
  description = "Run QEMU without a display window."
  default     = true
}

source "qemu" "fps" {
  # The cloud image is a ready disk, so the build boots it as it is.
  iso_url      = var.base_image_url
  iso_checksum = var.base_image_checksum
  disk_image   = true
  # Make a full copy and not a backing file, so the output stands alone.
  use_backing_file = false

  vm_name          = "fps-base.qcow2"
  output_directory = var.output_directory
  format           = "qcow2"
  disk_size        = var.disk_size
  disk_interface   = "virtio"
  disk_compression = true
  net_device       = "virtio-net"

  accelerator = var.accelerator
  memory      = var.memory
  cpus        = var.cpus
  headless    = var.headless

  # cloud-init reads the build user from this NoCloud seed (a CD labeled cidata).
  cd_label = "cidata"
  cd_content = {
    "meta-data" = "instance-id: fps-image-build\nlocal-hostname: fps-image-build\n"
    "user-data" = templatefile("${path.root}/cloud-init/build-user-data.tftpl", { password = var.ssh_password })
  }

  communicator = "ssh"
  ssh_username = "packer"
  ssh_password = var.ssh_password
  ssh_timeout  = var.ssh_timeout

  # Remove the build user and its sudo rule, then power off.
  shutdown_command = "sudo sh -c 'rm -f /etc/sudoers.d/90-cloud-init-users; userdel -rf packer; shutdown -P now'"
}

build {
  name    = "fps-base"
  sources = ["source.qemu.fps"]

  provisioner "shell" {
    inline = ["mkdir -p /tmp/fps-stage/files /tmp/fps-stage/final /tmp/fps-stage/modules"]
  }

  provisioner "file" {
    source      = var.controller_binary
    destination = "/tmp/fps-stage/fps-controller"
  }

  provisioner "file" {
    source      = var.agent_binary
    destination = "/tmp/fps-stage/fps-agent"
  }

  provisioner "file" {
    source      = "${var.modules_dir}/"
    destination = "/tmp/fps-stage/modules/"
  }

  provisioner "file" {
    source      = "${path.root}/files/"
    destination = "/tmp/fps-stage/files/"
  }

  provisioner "file" {
    source      = "${path.root}/final/"
    destination = "/tmp/fps-stage/final/"
  }

  provisioner "shell" {
    execute_command = "sudo -E bash '{{ .Path }}'"
    script          = "${path.root}/scripts/provision.sh"
  }

  provisioner "shell" {
    execute_command = "sudo -E bash '{{ .Path }}'"
    script          = "${path.root}/scripts/cleanup.sh"
  }
}
