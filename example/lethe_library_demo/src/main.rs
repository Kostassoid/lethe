use lethe::core::storage::StorageRef;
use lethe::core::storage::System;

fn main() {
    let storage_devices = System::get_storage_devices().expect("Something went wrong");

    for s in storage_devices {
        println!("Found device: {}", s.id());
    }
}
