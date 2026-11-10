# frozen_string_literal: true

# Reproducible integration-test VMs for Lethe.
#
# Every machine boots with an additional 1 GiB scratch disk. The guest test
# harness only wipes that disk, never the boot disk. Linux and Windows guests
# use QEMU through the `vagrant-qemu` plugin. macOS guests use Apple's
# Virtualization.framework through Tart and the `vagrant-tart` plugin, which
# only works on Apple Silicon Macs.
#
# Typical usage:
#
#   vagrant plugin install vagrant-qemu
#   vagrant up linux-x64 --provider=qemu
#   vagrant ssh linux-x64 -c "cd ~/lethe && bash testing/run-tests.sh"
#   vagrant destroy -f linux-x64
#
# On Apple Silicon macOS:
#
#   brew install openai/tools/tart
#   vagrant plugin install vagrant-tart
#   vagrant up macos-arm64 --provider=tart
#
# Machine names:
#   linux-x64, linux-arm64, windows-x64, windows-arm64, macos-arm64

require "fileutils"

Vagrant.require_version ">= 2.4.0"

def windows_host?
  RUBY_PLATFORM.match?(/(mingw|mswin|msvcrt)/)
end

def darwin_host?
  RUBY_PLATFORM.include?("darwin")
end

def apple_silicon_host?
  darwin_host? && RUBY_PLATFORM.match?(/(arm64|aarch64)/)
end

def qemu_binary_names
  %w[qemu-system-x86_64 qemu-system-aarch64]
end

def qemu_file_present?(dir, name)
  extensions = windows_host? ? [".exe", ""] : [""]
  extensions.any? { |ext| File.file?(File.join(dir, "#{name}#{ext}")) }
end

def qemu_directory?(dir)
  qemu_file_present?(dir, "qemu-img") &&
    qemu_binary_names.any? { |name| qemu_file_present?(dir, name) }
end

def qemu_search_dirs
  dirs = []

  # Accept both an installation directory and a path to the executable.
  [ENV["QEMU_BIN"], ENV["QEMU_DIR"]].compact.each do |value|
    dirs << (File.directory?(value) ? value : File.dirname(value))
  end

  if windows_host?
    dirs.concat([
      "C:/Program Files/qemu",
      "C:/Program Files (x86)/qemu",
      "C:/tools/qemu",
      "C:/msys64/mingw64/bin",
      "C:/msys64/ucrt64/bin",
      File.join(ENV.fetch("ProgramData", "C:/ProgramData"), "chocolatey", "bin"),
      File.join(ENV.fetch("LOCALAPPDATA", ""), "Programs", "qemu")
    ])
  elsif darwin_host?
    dirs.concat(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"])
  else
    dirs.concat(["/usr/bin", "/usr/local/bin"])
  end

  dirs.reject { |dir| dir.nil? || dir.empty? }.uniq
end

def find_qemu_dir
  path_dirs = ENV.fetch("PATH", "").split(File::PATH_SEPARATOR)
  (path_dirs + qemu_search_dirs).find { |dir| qemu_directory?(dir) }
end

def find_qemu_firmware_dir(qemu_dir)
  [
    File.join(qemu_dir, "share", "qemu"),
    File.join(qemu_dir, "share"),
    qemu_dir,
    "/usr/share/qemu",
    "/usr/local/share/qemu",
    "/opt/homebrew/share/qemu"
  ].find { |dir| File.file?(File.join(dir, "edk2-aarch64-code.fd")) }
end

MACHINE_NAMES = %w[linux-x64 linux-arm64 windows-x64 windows-arm64 macos-arm64].freeze
REQUESTED_MACHINES = ARGV.select { |arg| MACHINE_NAMES.include?(arg) }.freeze

qemu_plugin_available = Vagrant.has_plugin?("vagrant-qemu")
tart_plugin_available = Vagrant.has_plugin?("vagrant-tart")
macos_vm_available = apple_silicon_host? && tart_plugin_available

qemu_dir = qemu_plugin_available ? find_qemu_dir : nil
qemu_available = !qemu_dir.nil?

if qemu_dir
  path_parts = ENV.fetch("PATH", "").split(File::PATH_SEPARATOR)
  ENV["PATH"] = ([qemu_dir] + path_parts).join(File::PATH_SEPARATOR) unless path_parts.include?(qemu_dir)

  # vagrant-qemu reads QEMU_DIR for the aarch64 firmware files. Point it at the
  # directory that actually contains them when one can be found.
  firmware_dir = find_qemu_firmware_dir(qemu_dir)
  if firmware_dir && !File.file?(File.join(ENV.fetch("QEMU_DIR", ""), "edk2-aarch64-code.fd"))
    ENV["QEMU_DIR"] = firmware_dir
  end
end

qemu_requested = REQUESTED_MACHINES.any? { |machine| machine != "macos-arm64" }
macos_requested = REQUESTED_MACHINES.include?("macos-arm64")

if qemu_requested && !qemu_available
  abort <<~MSG
    QEMU and the vagrant-qemu plugin are required for Linux and Windows test VMs.

      vagrant plugin install vagrant-qemu

    Install QEMU and make sure `qemu-img` and a `qemu-system-*` binary are on
    PATH, or set QEMU_DIR to the directory containing those executables.
  MSG
end

if macos_requested && !macos_vm_available
  abort <<~MSG
    The macos-arm64 test VM requires an Apple Silicon Mac with Tart and the
    vagrant-tart plugin:

      brew install cirruslabs/cli/tart
      vagrant plugin install vagrant-tart
  MSG
end

if !qemu_available && !macos_vm_available
  abort <<~MSG
    No usable VM provider was found.

    For Linux and Windows guests:
      vagrant plugin install vagrant-qemu

    For macOS guests on Apple Silicon:
      brew install openai/tools/tart
      vagrant plugin install vagrant-tart
  MSG
end

ROOT_DIR = File.expand_path(__dir__)
CACHE_DIR = File.join(ROOT_DIR, "testing", ".cache")
SOURCE_ARCHIVE = File.join(CACHE_DIR, "lethe-src.tar.gz")

# Ship the current working tree to each guest. This keeps the guests
# independent from host shared-folder support, which is limited and
# platform-specific in the QEMU provider.
if (ARGV & %w[up provision reload resume validate]).any? && ENV["LETHE_SKIP_SOURCE_ARCHIVE"] != "1"
  FileUtils.mkdir_p(CACHE_DIR)

  excludes = %w[./.git ./target ./.vagrant ./testing/.cache ./.idea]
  args = ["-czf", SOURCE_ARCHIVE]
  excludes.each { |path| args.concat(["--exclude", path]) }
  args.concat(["-C", ROOT_DIR, "."])

  abort("Unable to create source archive #{SOURCE_ARCHIVE}") unless system("tar", *args)
end

TEST_DISK_SIZE = ENV.fetch("LETHE_TEST_DISK_SIZE", "1GB")

MACOS_SCRATCH_DISK = File.join(CACHE_DIR, "macos-scratch.img")
MACOS_SCRATCH_BYTES = 1 << 30

MACOS_IMAGE = ENV.fetch("LETHE_MACOS_IMAGE", "ghcr.io/cirruslabs/macos-sequoia-base:latest")
# Use bridged networking when the host VPN blocks the default vmnet NAT (for
# example Mullvad). The guest then gets an address directly from the LAN.
MACOS_NET_BRIDGED = ENV["LETHE_MACOS_NET_BRIDGED"]
# Override SSH address discovery when using bridged networking: ARP entries
# expire when the guest is idle, which makes `tart ip --resolver arp` flaky.
MACOS_SSH_HOST = ENV["LETHE_MACOS_SSH_HOST"]
# vagrant-tart always calls `tart set --disk-size`, and Tart refuses to shrink a
# disk. Keep the configured size in sync with the selected image: the base and
# vanilla images use a 50 GB virtual disk, the xcode images use 140 GB.
MACOS_DISK_GB = Integer(
  ENV.fetch("LETHE_MACOS_DISK_GB", MACOS_IMAGE.match?(/xcode/) ? "140" : "50")
)

if macos_vm_available
  FileUtils.mkdir_p(CACHE_DIR)

  unless File.file?(MACOS_SCRATCH_DISK) && File.size(MACOS_SCRATCH_DISK) == MACOS_SCRATCH_BYTES
    File.open(MACOS_SCRATCH_DISK, "wb") { |file| file.truncate(MACOS_SCRATCH_BYTES) }
  end
end

def qemu_acceleration(guest_arch)
  accel = ENV["LETHE_QEMU_ACCEL"]

  # WHPX is unreliable on many Windows hosts (and often unavailable inside
  # VMs), so default to software emulation there. Set LETHE_QEMU_ACCEL=whpx
  # to opt into hardware acceleration.
  accel = "tcg" if accel.nil? && windows_host?
  return nil if accel.nil?

  machine = if guest_arch == "x86_64"
              "q35,accel=#{accel}"
            else
              "virt,highmem=on,accel=#{accel}"
            end

  { machine: machine, cpu: accel == "tcg" ? "max" : "host" }
end

def configure_qemu(machine, guest_arch, ssh_port)
  machine.vm.provider "qemu" do |qe|
    qe.arch = guest_arch
    qe.memory = ENV.fetch("LETHE_VM_MEMORY", "4G")
    qe.smp = ENV.fetch("LETHE_VM_CPUS", "2")
    qe.ssh_port = ssh_port
    qe.ssh_auto_correct = true

    acceleration = qemu_acceleration(guest_arch)
    if acceleration
      qe.machine = acceleration[:machine]
      qe.cpu = acceleration[:cpu]
    end

    if windows_host?
      # The Windows QEMU build has neither daemonize mode nor Unix sockets.
      qe.no_daemonize = true
      qe.control_port = ssh_port + 10
      qe.debug_port = ssh_port + 20
    end
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

  if qemu_available
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

  if macos_vm_available
    config.vm.define "macos-arm64", autostart: false do |machine|
      machine.vm.provider "tart" do |tart|
        # The base image includes the Command Line Tools needed to build Lethe.
        # The xcode variant is several times larger and is only needed when a
        # full Xcode installation is required.
        tart.image = MACOS_IMAGE
        tart.name = ENV.fetch("LETHE_MACOS_VM_NAME", "lethe-macos-arm64")
        tart.cpus = Integer(ENV.fetch("LETHE_MACOS_CPUS", ENV.fetch("LETHE_VM_CPUS", "2")))
        tart.memory = Integer(ENV.fetch("LETHE_MACOS_MEMORY_MB", "8192"))
        tart.disk = MACOS_DISK_GB
        tart.gui = false
        run_args = ["--disk", MACOS_SCRATCH_DISK]
        if MACOS_NET_BRIDGED && !MACOS_NET_BRIDGED.empty?
          run_args << "--net-bridged=#{MACOS_NET_BRIDGED}"
          # Bridged guests are not in the host's vmnet DHCP lease file.
          tart.ip_resolver = "arp"
        end
        tart.extra_run_args = run_args
      end

      machine.vm.communicator = "ssh"
      # The first boot of a freshly cloned macOS image can take well over the
      # global boot timeout (first-boot setup and Spotlight indexing).
      machine.vm.boot_timeout = Integer(ENV.fetch("LETHE_MACOS_BOOT_TIMEOUT", "1800"))
      machine.ssh.username = ENV.fetch("LETHE_MACOS_USER", "admin")
      machine.ssh.password = ENV.fetch("LETHE_MACOS_PASSWORD", "admin")
      machine.ssh.host = MACOS_SSH_HOST if MACOS_SSH_HOST && !MACOS_SSH_HOST.empty?
      machine.vm.provision "file", source: SOURCE_ARCHIVE, destination: "lethe-src.tar.gz"
      machine.vm.provision "shell", path: "testing/provision-macos.sh"
    end
  end
end
