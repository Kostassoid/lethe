test_drive_path = './testing/test_drive.vdi'

Vagrant.configure("2") do |config|

  config.vm.provider "virtualbox" do |v|
    v.memory = 1024
    v.cpus = 2

    v.customize ["modifyvm", :id, "--usb", "on"]
    v.customize ["modifyvm", :id, "--usbehci", "off"]
    v.customize ["modifyvm", :id, "--natdnshostresolver1", "on"]
    v.customize ["modifyvm", :id, "--natdnsproxy1", "on"]

    unless File.exist?(test_drive_path)
      v.customize ['createhd', '--filename', test_drive_path, '--size', 99]
    end
    v.customize ['storageattach', :id, '--storagectl', 'SATA', '--port', 1, '--device', 0, '--type', 'hdd', '--medium', test_drive_path]

  end

  config.vm.define "linux" do |linux|
    linux.vm.box = "minimal/trusty64"

    linux.vm.provision "shell", inline: "echo Welcome to testing Lethe"
    linux.vm.provision "shell", inline: "apt-get update"
    linux.vm.provision "shell", inline: "apt-get -y install curl"
    linux.vm.provision "shell", inline: "curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal"
  end

  config.vm.define "macos" do |macos|
    macos.vm.box = "gobadiah/macos-mojave"
  end

end
