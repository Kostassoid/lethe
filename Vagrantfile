# frozen_string_literal: true

# Reproducible integration-test VMs for Lethe.
#
# Every machine boots with an additional 1 GiB scratch disk named
# `lethe_test`. The guest test harness only wipes that disk, never the boot
# disk. QEMU is used on every host OS through the `vagrant-qemu` plugin.
#
# Typical usage:
#
#   vagrant plugin install vagrant-qemu
#   vagrant up linux-x64 --provider=qemu
#   vagrant ssh linux-x64 -c "cd ~/lethe && bash testing/run-tests.sh"
#   vagrant destroy -f linux-x64
#
# Machine names:
#   linux-x64, linux-arm64, windows-x64, windows-arm64
#
# macOS guests are not offered here: Apple's EULA only permits macOS
# virtualization on Apple-branded hardware and there is no QEMU/libvirt
# Vagrant box. Run `testing/run-tests.sh` on a real Mac instead.

require "fileutils"

Vagrant.require_version ">= 2.4.0"

unless Vagrant.has_plugin?("vagrant-qemu")
  abort <<~MSG
    The vagrant-qemu provider plugin is required.

      vagrant plugin install vagrant-qemu

    QEMU must also be installed and available on PATH.
  MSG
end

ROOT_DIR = File.expand_path(__dir__)
CACHE_DIR = File.join(ROOT_DIR, "testing", ".cache")
SOURCE_ARCHIVE = File.join(CACHE_DIR, "lethe-src.tar.gz")

# Ship the current working tree to each guest. This keeps the guests
# independent from host shared-folder support, which is limited and
# platform-specific in the QEMU provider.
if (ARGV & %w[up provision reload resume]).any? && ENV["LETHE_SKIP_SOURCE_ARCHIVE"] != "1"
  FileUtils.mkdir_p(CACHE_DIR)

  excludes = %w[./.git ./target ./.vagrant ./testing/.cache ./.idea]
  args = ["-czf", SOURCE_ARCHIVE]
  excludes.each { |path| args.concat(["--exclude", path]) }
  args.concat(["-C", ROOT_DIR, "."])

  abort("Unable to create source archive #{SOURCE_ARCHIVE}") unless system("tar", *args)
end

TEST_DISK_SIZE = ENV.fetch("LETHE_TEST_DISK_SIZE", "1GB")

def configure_qemu(machine, guest_arch, ssh_port)
  machine.vm.provider "qemu" do |qe|
    qe.arch = guest_arch
    qe.memory = ENV.fetch("LETHE_VM_MEMORY", "4G")
    qe.smp = ENV.fetch("LETHE_VM_CPUS", "2")
    qe.ssh_port = ssh_port
    qe.ssh_auto_correct = true
  end
end

linux_x64_box = ENV.fetch("LETHE_LINUX_X64_BOX", "cloud-image/ubuntu-24.04")
linux_arm64_box = ENV.fetch("LETHE_LINUX_ARM64_BOX", "cloud-image/ubuntu-24.04")
windows_x64_box = ENV.fetch("LETHE_WINDOWS_X64_BOX", "gusztavvargadr/windows-server-2025-standard")
windows_arm64_box = ENV["LETHE_WINDOWS_ARM64_BOX"]
windows_arm64_requested = ARGV.any? { |arg| arg == "windows-arm64" }

if windows_arm64_box.nil? && windows_arm64_requested
  abort <<~MSG
    No Vagrant box with a QEMU/libvirt provider is published for Windows
    ARM64. Provide one and point LETHE_WINDOWS_ARM64_BOX at it, for example:

      LETHE_WINDOWS_ARM64_BOX=my/windows-11-arm64 vagrant up windows-arm64 --provider=qemu

    The box must support the libvirt/QEMU provider and SSH.
  MSG
end

Vagrant.configure("2") do |config|
  config.vm.box_check_update = false
  config.vm.boot_timeout = Integer(ENV.fetch("LETHE_VM_BOOT_TIMEOUT", "900"))
  config.vm.synced_folder ".", "/vagrant", disabled: true

  config.vm.define "linux-x64", autostart: false do |machine|
    machine.vm.box = linux_x64_box
    machine.vm.box_architecture = "amd64"
    machine.vm.disk :disk, size: TEST_DISK_SIZE, name: "lethe_test"
    configure_qemu(machine, "x86_64", 50_022)
    machine.vm.provision "file", source: SOURCE_ARCHIVE, destination: "lethe-src.tar.gz"
    machine.vm.provision "shell", path: "testing/provision-linux.sh"
  end

  config.vm.define "linux-arm64", autostart: false do |machine|
    machine.vm.box = linux_arm64_box
    machine.vm.box_architecture = "arm64"
    machine.vm.disk :disk, size: TEST_DISK_SIZE, name: "lethe_test"
    configure_qemu(machine, "aarch64", 50_023)
    machine.vm.provision "file", source: SOURCE_ARCHIVE, destination: "lethe-src.tar.gz"
    machine.vm.provision "shell", path: "testing/provision-linux.sh"
  end

  config.vm.define "windows-x64", autostart: false do |machine|
    machine.vm.box = windows_x64_box
    machine.vm.box_architecture = "amd64"
    machine.vm.communicator = "ssh"
    machine.vm.disk :disk, size: TEST_DISK_SIZE, name: "lethe_test"
    configure_qemu(machine, "x86_64", 50_024)
    machine.vm.provision "file", source: SOURCE_ARCHIVE, destination: "lethe-src.tar.gz"
    machine.vm.provision "shell", path: "testing/provision-windows.ps1", privileged: false
  end

  if windows_arm64_box
    config.vm.define "windows-arm64", autostart: false do |machine|
      machine.vm.box = windows_arm64_box
      machine.vm.box_architecture = "arm64"
      machine.vm.communicator = "ssh"
      machine.vm.disk :disk, size: TEST_DISK_SIZE, name: "lethe_test"
      configure_qemu(machine, "aarch64", 50_025)
      machine.vm.provision "file", source: SOURCE_ARCHIVE, destination: "lethe-src.tar.gz"
      machine.vm.provision "shell", path: "testing/provision-windows.ps1", privileged: false
    end
  end
end
