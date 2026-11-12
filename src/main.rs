#![recursion_limit = "256"]

use anyhow::{Context, Result, anyhow};
use indicatif::HumanBytes;
use prettytable::{Attr, Cell, Row, Table, format, row};

mod actions;
mod power;
mod sanitization;
mod storage;
mod ui;

use actions::{Percent, Verification, WipePlan};
use power::SleepInhibitor;
use sanitization::SchemeRepo;
use storage::{StorageDevice, StorageReadiness, StorageRef, System};
use ui::{args, cli, storage_repo};

fn main() -> Result<()> {
    let schemes = SchemeRepo::default();
    let mut app = args::build_command(&schemes);

    match app.get_matches_mut().subcommand() {
        Some(("list", _)) => {
            let storage_repo = enumerate_storage_repo();

            let mut t = Table::new();
            t.set_format(*format::consts::FORMAT_CLEAN);
            t.set_titles(row![
                "Device ID",
                "Short ID",
                "Size",
                "Type",
                "Label",
                "Mount Point",
            ]);

            let format_device = |tt: &mut Table, x: &StorageRef, level: usize| {
                let short_id = storage_repo.get_short_id(&x.id).map_or("", String::as_str);
                let device_id = format!("{}{}", " ".repeat(level * 2), x.id);

                let device_cell = Cell::new(&device_id).with_style(Attr::Bold);
                let short_id_cell = Cell::new(short_id).with_style(Attr::Bold);

                let row = match &x.readiness {
                    StorageReadiness::Ready(details) => Row::new(vec![
                        device_cell,
                        short_id_cell,
                        Cell::new(&HumanBytes(details.size).to_string()),
                        Cell::new(&details.storage_type.to_string()),
                        Cell::new(details.label.as_deref().unwrap_or("")),
                        Cell::new(details.mount_point.as_deref().unwrap_or("")),
                    ]),
                    StorageReadiness::Locked => Row::new(vec![
                        device_cell,
                        short_id_cell,
                        Cell::new(&HumanBytes(0).to_string()),
                        Cell::new("locked"),
                        Cell::new("locked"),
                        Cell::new("locked"),
                    ]),
                };
                tt.add_row(row);
            };

            let devices = storage_repo.devices();
            if devices.is_empty() {
                eprintln!(
                    "No devices found! Are you running the application with root/administrator permissions?"
                );
                std::process::exit(1);
            }

            for x in storage_repo.devices() {
                format_device(&mut t, x, 0);
                for c in &x.children {
                    format_device(&mut t, c, 1);
                }
            }
            t.printstd();
        }
        Some(("wipe", cmd)) => {
            let storage_repo = enumerate_storage_repo();

            let device_id = cmd
                .get_one::<String>("device")
                .ok_or_else(|| anyhow!("Invalid device ID"))?;
            let device = storage_repo
                .find_by_id(device_id)
                .ok_or_else(|| anyhow!("Unknown device {device_id}"))?;

            let StorageReadiness::Ready(device_details) = &device.readiness else {
                Err(anyhow!("Device {device_id} is locked."))?
            };

            let scheme_id = cmd.get_one::<String>("scheme").unwrap();
            let scheme = schemes
                .find(scheme_id)
                .ok_or_else(|| anyhow!("Unknown scheme {scheme_id}"))?;

            let coverage_value = cmd.get_one::<f32>("coverage").unwrap();
            let coverage = Percent::new(*coverage_value).context("Invalid coverage argument")?;

            let verification = match cmd.get_one::<String>("verify").unwrap().as_str() {
                "no" => Verification::No,
                "all" => Verification::All(coverage),
                _ => Verification::Last(coverage),
            };

            let block_size_arg = cmd.get_one::<String>("blocksize").unwrap();
            let block_size = args::parse_block_size(block_size_arg)
                .with_context(|| format!("Invalid blocksize value: {block_size_arg}"))?;

            let offset_arg = cmd.get_one::<String>("offset").unwrap();
            let offset: u64 = args::parse_bytes(offset_arg)
                .with_context(|| format!("Invalid offset value: {offset_arg}"))?;

            let retries = cmd
                .get_one::<u32>("retries")
                .ok_or_else(|| anyhow!("Invalid retries number value"))?;

            let queue_depth = cmd
                .get_one::<u32>("queue-depth")
                .ok_or_else(|| anyhow!("Invalid queue depth value"))?;
            let queue_depth = usize::try_from((*queue_depth).max(1))
                .context("Queue depth does not fit into addressable memory")?;

            let plan = WipePlan::new(
                scheme.clone(),
                verification,
                offset..device_details.size,
                block_size,
            )?;

            let _sleep_inhibitor = SleepInhibitor::acquire(&format!("Wiping device {device_id}"))
                .inspect_err(|err| {
                    eprintln!("Warning: unable to prevent the system from sleeping: {err:#}");
                })
                .ok();

            if queue_depth <= 1 {
                let session = cli::wipe_session(&device.id, cmd.get_flag("yes"), 1);
                let device_access = device.access()?;
                plan.execute(device_access, Box::new(session), *retries)?;
            } else {
                let session = cli::wipe_session(&device.id, cmd.get_flag("yes"), queue_depth);
                let device_access = device.access_async(queue_depth)?;
                plan.execute_async(device_access, Box::new(session), *retries, queue_depth)?;
            }
        }
        _ => {
            app.print_help()?;
            std::process::exit(1)
        }
    }

    Ok(())
}

/// Discovers storage devices, or exits with a user-facing explanation. Device discovery
/// requires elevated privileges on most platforms.
fn enumerate_storage_repo() -> storage_repo::StorageRepo {
    let storage_devices = System::enumerate_storage_devices().unwrap_or_else(|err| {
        eprintln!("Unable to enumerate storage devices. {err:#}");

        if cfg!(target_os = "linux") {
            let is_wsl =
                std::fs::read_to_string("/proc/version").is_ok_and(|v| v.contains("Microsoft"));

            if is_wsl {
                eprintln!("WSL is not supported.");
            }
        }

        std::process::exit(1);
    });

    storage_repo::StorageRepo::from(storage_devices)
}
