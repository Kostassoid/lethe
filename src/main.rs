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
use storage::{StorageDetails, StorageDevice, StorageReadiness, StorageRef, StorageType, System};
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
                "Bus",
                "Drive",
                "QD",
                "Erase",
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
                        Cell::new(&details.bus.to_string()),
                        Cell::new(&details.drive_kind.to_string()),
                        Cell::new(&details.recommended_queue_depth().to_string()),
                        Cell::new(&hardware_erase_text(details)),
                        Cell::new(details.label.as_deref().unwrap_or("")),
                        Cell::new(details.mount_point.as_deref().unwrap_or("")),
                    ]),
                    StorageReadiness::Locked => Row::new(vec![
                        device_cell,
                        short_id_cell,
                        Cell::new(&HumanBytes(0).to_string()),
                        Cell::new("locked"),
                        Cell::new("-"),
                        Cell::new("-"),
                        Cell::new("-"),
                        Cell::new("-"),
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
            print_erase_legend();
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

            let queue_depth = match cmd
                .get_one::<args::QueueDepth>("queue-depth")
                .ok_or_else(|| anyhow!("Invalid queue depth value"))?
            {
                args::QueueDepth::Auto => device_details.recommended_queue_depth(),
                args::QueueDepth::Fixed(depth) => *depth,
            };

            let plan = WipePlan::new(
                scheme.clone(),
                verification,
                offset..device_details.size,
                block_size,
            )?;

            let hardware_erase = *cmd
                .get_one::<args::HardwareEraseMethod>("hardware-erase")
                .ok_or_else(|| anyhow!("Invalid hardware erase value"))?;

            if hardware_erase != args::HardwareEraseMethod::Off && offset != 0 {
                return Err(anyhow!(
                    "Hardware erase operates on the whole drive and cannot be combined with --offset."
                ));
            }

            let hardware_erase = resolve_hardware_erase(device_details, hardware_erase)?;

            if plan.steps.is_empty() && hardware_erase == args::HardwareEraseMethod::Off {
                return Err(anyhow!(
                    "No software wipe passes were selected. Use --scheme=none together with --hardware-erase, or choose a wiping scheme."
                ));
            }

            let plan = plan.with_hardware_erase(hardware_erase);

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

fn resolve_hardware_erase(
    details: &StorageDetails,
    requested: args::HardwareEraseMethod,
) -> Result<args::HardwareEraseMethod> {
    if requested == args::HardwareEraseMethod::Off {
        return Ok(args::HardwareEraseMethod::Off);
    }

    if matches!(details.storage_type, StorageType::Partition) {
        return Err(anyhow!(
            "Hardware erase works on the whole drive; select the physical drive instead of a partition."
        ));
    }

    let capabilities = details.hardware_erase.ok_or_else(|| {
        anyhow!(
            "Hardware erase capabilities could not be probed. Run `lethe list` with administrator/root privileges."
        )
    })?;

    if requested == args::HardwareEraseMethod::Auto {
        let selected = if capabilities.nvme_sanitize() {
            args::HardwareEraseMethod::NvmeSanitize
        } else if capabilities.ata_secure_erase {
            args::HardwareEraseMethod::AtaSecure
        } else if capabilities.nvme_format {
            args::HardwareEraseMethod::NvmeFormat
        } else if capabilities.ata_sanitize() {
            args::HardwareEraseMethod::AtaSanitize
        } else {
            return Err(anyhow!(
                "No supported hardware erase method was reported for this device."
            ));
        };

        ensure_hardware_erase_implemented(selected)?;
        return Ok(selected);
    }

    let supported = match requested {
        args::HardwareEraseMethod::AtaSecure => capabilities.ata_secure_erase,
        args::HardwareEraseMethod::AtaSanitize => capabilities.ata_sanitize(),
        args::HardwareEraseMethod::NvmeSanitize => capabilities.nvme_sanitize(),
        args::HardwareEraseMethod::NvmeFormat => capabilities.nvme_format,
        args::HardwareEraseMethod::Off | args::HardwareEraseMethod::Auto => unreachable!(),
    };

    if !supported {
        return Err(anyhow!(
            "{requested} is not reported as supported by this device."
        ));
    }

    ensure_hardware_erase_implemented(requested)?;
    Ok(requested)
}

fn ensure_hardware_erase_implemented(method: args::HardwareEraseMethod) -> Result<()> {
    if matches!(
        method,
        args::HardwareEraseMethod::AtaSanitize | args::HardwareEraseMethod::NvmeFormat
    ) {
        return Err(anyhow!("{method} is detected but not implemented yet."));
    }

    Ok(())
}

fn hardware_erase_text(details: &StorageDetails) -> String {
    if matches!(details.storage_type, StorageType::Partition) {
        return String::new();
    }

    match &details.hardware_erase {
        Some(capabilities) => capabilities.abbreviations(),
        None => "?".to_string(),
    }
}

fn print_erase_legend() {
    println!();
    println!("  * Hardware erase methods (whole disk only; partitions are left blank).");
    println!("    Methods are listed in the recommended order:");
    println!("    NS*  NVMe Sanitize — preferred for NVMe; covers the whole device");
    println!("    NF*  NVMe Format NVM — namespace-scoped fallback when Sanitize is unavailable");
    println!("    SA*  ATA SANITIZE — preferred for SATA; covers the whole device");
    println!("    SE*  ATA Secure Erase — older command; effective on many drives but varies");
    println!("    ?    Not probed or unsupported on this platform");
    println!("    -    No hardware erase method reported");
    println!("    Prefer NS over NF, and SA over SE, when both are listed.");
    println!("    Crypto erase is fastest, block erase physically erases, and overwrite writes");
    println!("    a pattern; the actual behavior depends on the drive firmware.");
    println!("    Execution is currently wired for NS* and SE*; SA* and NF* are detected only.");
    println!(
        "    Probing is currently implemented on Windows and may require administrator rights;"
    );
    println!("    USB bridges often need elevated pass-through access.");
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
