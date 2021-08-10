Vagrant.require_version ">= 2.2.8"
ENV['VAGRANT_EXPERIMENTAL'] = 'disks'

Vagrant.configure("2") do |config|
  config.vm.define "hashicorp" do |h|
    h.vm.box = "hashicorp/bionic64"
    h.vm.provider :virtualbox

    h.vm.disk :disk, size: "1GB", name: "extra_storage"

    h.vm.provision "shell", inline: "echo Welcome to testing Lethe"
    h.vm.provision "shell", inline: "apt-get update"
    h.vm.provision "shell", inline: "apt-get -y install curl build-essential"
    h.vm.provision "shell", inline: "curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal", privileged: false
  end
end
