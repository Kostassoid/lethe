#![recursion_limit = "256"]

#[macro_use]
extern crate anyhow;
use anyhow::{Context, Result};

extern crate clap;
use clap::{Arg, Command, SubCommand};

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
        .subcommand(SubCommand::with_name("list").about("list available storage devices"))
        .subcommand(
            SubCommand::with_name("wipe")
                .about("Wipe storage device")
                .after_help(schemes_explanation.as_str())
                .arg(
                    Arg::with_name("device")
                        .required(true)
                        .takes_value(true)
                        .index(1)
                        .help("Storage device ID"),
                )
                .arg(
                    Arg::with_name("scheme")
                        .long("scheme")
                        .short('s')
                        .takes_value(true)
                        .possible_values(&scheme_keys)
                        .default_value("random2x")
                        .help("Data sanitization scheme"),
                )
                .arg(
                    Arg::with_name("verify")
                        .long("verify")
                        .short('v')
                        .takes_value(true)
                        .possible_values(&["no", "last", "all"])
                        .default_value("last")
                        .help("Verify after completion"),
                )
                .arg(
                    Arg::with_name("blocksize")
                        .long("blocksize")
                        .short('b')
                        .takes_value(true)
                        .default_value("1m")
                        .help("Block size"),
                )
                .arg(
                    Arg::with_name("offset")
                        .long("offset")
                        .short('o')
                        .takes_value(true)
                        .default_value("0")
                        .help("Starting offset (in bytes)"),
                )
                .arg(
                    Arg::with_name("retries")
                        .long("retries")
                        .short('r')
                        .takes_value(true)
                        .default_value("8")
                        .help("Maximum number of retries"),
                )
                .arg(
                    Arg::with_name("yes")
                        .long("yes")
                        .short('y')
                        .help("Automatically confirm"),
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
                tt.add_row(row![
                    style(format!("{}{}", " ".repeat(level * 2), &x.id)).bold(),
                    style(storage_repo.get_short_id(&x.id).unwrap_or(&"".to_owned())).bold(),
                    HumanBytes(x.details.size),
                    &x.details.storage_type,
                    (&x.details.label).as_ref().unwrap_or(&"".to_string()),
                    (&x.details.mount_point).as_ref().unwrap_or(&"".to_string()),
                ]);
            };

            let devices = storage_repo.devices();
            if devices.is_empty() {
                eprintln!("No devices found! Are you running the application with root/administrator permissions?");
                std::process::exit(1);
            }

            for x in storage_repo.devices() {
                format_device(&mut t, &x, 0);
                for c in &x.children {
                    format_device(&mut t, &c, 1);
                }
            }
            t.printstd();
        }
        Some(("wipe", cmd)) => {
            let device_id = cmd.value_of("device").ok_or(anyhow!("Invalid device ID"))?;
            let scheme_id = cmd.value_of("scheme").unwrap();
            let verification = match cmd.value_of("verify").unwrap() {
                "no" => Verify::No,
                "last" => Verify::Last,
                "all" => Verify::All,
                _ => Verify::Last,
            };
            let block_size_arg = cmd.value_of("blocksize").unwrap();
            let block_size = ui::args::parse_block_size(block_size_arg)
                .context(format!("Invalid blocksize value: {}", block_size_arg))?;

            let offset_arg = cmd.value_of("offset").unwrap();
            let offset: u64 = ui::args::parse_bytes(offset_arg)
                .context(format!("Invalid offset value: {}", offset_arg))?;

            let device = storage_repo
                .find_by_id(device_id)
                .ok_or(anyhow!("Unknown device {}", device_id))?;
            let scheme = schemes
                .find(scheme_id)
                .ok_or(anyhow!("Unknown scheme {}", scheme_id))?;

            let retries = cmd
                .value_of("retries")
                .unwrap()
                .parse()
                .context("Invalid retries number value")?;

            let task = WipeTask::new(
                scheme.clone(),
                verification,
                device.details.size,
                block_size,
                offset,
            )?;

            let mut state = WipeState::default();
            state.retries_left = retries;

            let mut session = frontend.wipe_session(&device.id, cmd.is_present("yes"));
            session.handle(&task, &state, WipeEvent::Created);

            match device.access() {
                Ok(mut access) => {
                    if !task.run(access.as_mut(), &mut state, &mut session) {
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
            // println!("{}", app.render_usage());
            std::process::exit(1)
        }
    }

    Ok(())
}
