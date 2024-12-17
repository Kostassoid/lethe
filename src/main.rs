#![recursion_limit = "256"]

#[macro_use]
extern crate anyhow;
use anyhow::{Context, Result};

extern crate clap;
use clap::{value_parser, Arg, ArgAction, Command};

#[macro_use]
extern crate prettytable;
use prettytable::{format, Table};

#[cfg(target_os = "macos")]
#[macro_use]
extern crate serde_derive;

#[cfg(target_os = "macos")]
extern crate plist;

use ::console::style;
use indicatif::HumanBytes;

mod storage;
use storage::*;

mod sanitization;
use sanitization::*;

mod actions;
use actions::*;

mod ui;
use ui::*;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn main() -> Result<()> {
    let schemes = SchemeRepo::default();
    let scheme_keys: Vec<_> = schemes.all().keys().cloned().collect();
    let schemes_explanation = cli::ConsoleFrontend::explain_schemes(&schemes);

    let mut app = Command::new("Lethe")
        .version(VERSION)
        .author("https://github.com/Kostassoid/lethe")
        .about("Secure disk wipe")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("list available storage devices"))
        .subcommand(
            Command::new("wipe")
                .about("Wipe storage device")
                .after_help(schemes_explanation)
                .arg(
                    Arg::new("device")
                        .required(true)
                        .num_args(1)
                        .index(1)
                        .help("Storage device ID"),
                )
                .arg(
                    Arg::new("scheme")
                        .long("scheme")
                        .short('s')
                        .num_args(1)
                        .value_parser(scheme_keys)
                        .default_value("random2x")
                        .help("Data sanitization scheme"),
                )
                .arg(
                    Arg::new("verify")
                        .long("verify")
                        .short('v')
                        .num_args(1)
                        .value_parser(["no", "last", "all"])
                        .default_value("last")
                        .help("Verify after completion"),
                )
                .arg(
                    Arg::new("coverage")
                        .long("coverage")
                        .short('c')
                        .help("Verification coverage (in percents)")
                        .num_args(1)
                        .value_parser(value_parser!(f32))
                        .default_value("100.0"),
                )
                .arg(
                    Arg::new("blocksize")
                        .long("blocksize")
                        .short('b')
                        .num_args(1)
                        .default_value("1m")
                        .help("Block size"),
                )
                .arg(
                    Arg::new("offset")
                        .long("offset")
                        .short('o')
                        .num_args(1)
                        .default_value("0")
                        .help("Starting offset (in bytes)"),
                )
                .arg(
                    Arg::new("retries")
                        .long("retries")
                        .short('r')
                        .num_args(1)
                        .value_parser(value_parser!(u32))
                        .default_value("8")
                        .help("Maximum number of retries"),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .short('y')
                        .help("Automatically confirm")
                        .action(ArgAction::SetTrue),
                ),
        );

    let storage_devices = System::enumerate_storage_devices().unwrap_or_else(|err| {
        eprintln!("Unable to enumerate storage devices. {:#}", err);

        if cfg!(linux) {
            let is_wsl = std::fs::read_to_string("/proc/version")
                .map(|v| v.contains("Microsoft"))
                .unwrap_or(false);

            if is_wsl {
                eprintln!("WSL is not supported.");
            }
        }

        std::process::exit(1);
    });
    let storage_repo = storage_repo::StorageRepo::from(storage_devices);

    let frontend = cli::ConsoleFrontend::new();

    match app.get_matches_mut().subcommand() {
        Some(("list", _)) => {
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
                match x.readiness {
                    StorageReadiness::Ready(ref details) =>
                        tt.add_row(row![
                        style(format!("{}{}", " ".repeat(level * 2), &x.id)).bold(),
                        style(storage_repo.get_short_id(&x.id).unwrap_or(&"".to_owned())).bold(),
                        HumanBytes(details.size),
                        &details.storage_type,
                        (&details.label).as_ref().unwrap_or(&"".to_string()),
                        (&details.mount_point).as_ref().unwrap_or(&"".to_string()),
                    ]),
                    StorageReadiness::Locked =>
                        tt.add_row(row![
                        style(format!("{}{}", " ".repeat(level * 2), &x.id)).bold(),
                        style(storage_repo.get_short_id(&x.id).unwrap_or(&"".to_owned())).bold(),
                        HumanBytes(0),
                        "locked",
                        "locked",
                        "locked",
                    ]),
                };
            };

            let devices = storage_repo.devices();
            if devices.is_empty() {
                eprintln!("No devices found! Are you running the application with root/administrator permissions?");
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
            let device_id = cmd
                .get_one::<String>("device")
                .ok_or(anyhow!("Invalid device ID"))?;
            let device = storage_repo
                .find_by_id(device_id)
                .ok_or(anyhow!("Unknown device {}", device_id))?;

            let StorageReadiness::Ready(device_details) = &device.readiness
            else { Err(anyhow!("Device {} is locked.", device_id))? };

            let scheme_id = cmd.get_one::<String>("scheme").unwrap();
            let scheme = schemes
                .find(scheme_id)
                .ok_or(anyhow!("Unknown scheme {}", scheme_id))?;

            let coverage_value = cmd.get_one::<f32>("coverage").unwrap();
            let coverage = Percent::new(*coverage_value).context("Invalid coverage argument")?;

            let verification = match cmd.get_one::<String>("verify").unwrap().as_str() {
                "no" => Verification::No,
                "last" => Verification::Last(coverage),
                "all" => Verification::All(coverage),
                _ => Verification::Last(coverage),
            };

            let block_size_arg = cmd.get_one::<String>("blocksize").unwrap();
            let block_size = args::parse_block_size(block_size_arg)
                .context(format!("Invalid blocksize value: {}", block_size_arg))?;

            let offset_arg = cmd.get_one::<String>("offset").unwrap();
            let offset: u64 = args::parse_bytes(offset_arg)
                .context(format!("Invalid offset value: {}", offset_arg))?;

            let retries = cmd
                .get_one::<u32>("retries")
                .ok_or(anyhow!("Invalid retries number value"))?;

            let task = WipeTask::new(
                scheme.clone(),
                verification,
                device_details.size,
                block_size,
                offset,
                *retries,
            )?;

            let mut state = (&task).into();

            let mut session = frontend.wipe_session(&device.id, cmd.get_flag("yes"));
            session.handle(&task, &state, WipeEvent::Created);

            match device.access() {
                Ok(mut storage) => {
                    if !task.run(storage.as_mut(), &mut state, &mut session) {
                        std::process::exit(1);
                    }
                }
                Err(err) => {
                    session.handle(&task, &state, WipeEvent::Fatal(err));
                    std::process::exit(1);
                }
            }
        }
        _ => {
            app.print_help()?;
            std::process::exit(1)
        }
    }

    Ok(())
}
