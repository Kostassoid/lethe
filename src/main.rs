#![recursion_limit = "256"]

use std::rc::Rc;

#[macro_use]
extern crate anyhow;
use anyhow::{Context, Result};

extern crate clap;
use clap::{App, AppSettings, Arg, SubCommand};

#[macro_use]
extern crate prettytable;
use format::FormatBuilder;
use prettytable::{format, Table};

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
    // ctrlc::set_handler(move || {
    // }).expect("Error setting Ctrl-C handler");

    let schemes = SchemeRepo::default();
    let scheme_keys: Vec<_> = schemes.all().keys().cloned().collect();

    let schemes_explanation = {
        let mut t = Table::new();
        let indent_table_format = FormatBuilder::new().padding(4, 1).build();
        t.set_format(indent_table_format);
        for (k, v) in schemes.all().iter() {
            let stages_count = v.stages.len();
            let passes = if stages_count != 1 { "passes" } else { "pass" };
            t.add_row(row![
                k,
                format!("{}, {} {}", v.description, stages_count, passes)
            ]);
        }
        format!("Data sanitization schemes:\n{}", t)
    };

    let app = App::new("Lethe")
        .version(VERSION)
        .author("https://github.com/Kostassoid/lethe")
        .about("Secure disk wipe")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .setting(AppSettings::VersionlessSubcommands)
        .subcommand(SubCommand::with_name("list").about("list available storage devices"))
        .subcommand(
            SubCommand::with_name("wipe")
                .about("Wipe storage device")
                .after_help(schemes_explanation.as_str())
                .arg(
                    Arg::with_name("device")
                        .long("device")
                        .short("d")
                        .required(true)
                        .takes_value(true)
                        .index(1)
                        .help("Storage device ID"),
                )
                .arg(
                    Arg::with_name("scheme")
                        .long("scheme")
                        .short("s")
                        .takes_value(true)
                        .possible_values(&scheme_keys)
                        .default_value("random2")
                        .help("Data sanitization scheme"),
                )
                .arg(
                    Arg::with_name("verify")
                        .long("verify")
                        .short("v")
                        .takes_value(true)
                        .possible_values(&["no", "last", "all"])
                        .default_value("last")
                        .help("Verify after completion"),
                )
                .arg(
                    Arg::with_name("blocksize")
                        .long("blocksize")
                        .short("bs")
                        .takes_value(true)
                        .default_value("64k")
                        .help("Block size"),
                )
                .arg(
                    Arg::with_name("yes")
                        .long("yes")
                        .short("y")
                        .help("Automatically confirm"),
                ),
        )
        .get_matches();

    let storage_devices = System::get_storage_devices()
        .unwrap_or_else(|err| {
            eprintln!("Unable to enumerate storage devices. {}", err);

            if cfg!(linux) {
                let is_wsl = std::fs::read_to_string("/proc/version")
                    .map(|v| v.contains("Microsoft"))
                    .unwrap_or(false);

                if is_wsl {
                    eprintln!("WSL is not supported at the moment as it doesn't provide direct storage device access.");
                }
            }

            std::process::exit(1);
        });
    let frontend = cli::ConsoleFrontend::new();

    match app.subcommand() {
        ("list", _) => {
            let mut t = Table::new();
            t.set_format(*format::consts::FORMAT_CLEAN);
            t.set_titles(row![
                "Device ID",
                "Size",
                "Type",
                "Mount Point",
                "Block Size"
            ]);
            for x in storage_devices {
                t.add_row(row![
                    style(x.id()).bold(),
                    HumanBytes(x.details().size),
                    x.details().storage_type,
                    (x.details().mount_point)
                        .as_ref()
                        .unwrap_or(&"".to_string()),
                    HumanBytes(x.details().block_size as u64)
                ]);
            }
            t.printstd();
        }
        ("wipe", Some(cmd)) => {
            let device_id = cmd.value_of("device").unwrap();
            let scheme_id = cmd.value_of("scheme").unwrap();
            let verification = match cmd.value_of("verify").unwrap() {
                "no" => Verify::No,
                "last" => Verify::Last,
                "all" => Verify::All,
                _ => Verify::Last,
            };
            let block_size = ui::args::parse_block_size(cmd.value_of("blocksize").unwrap())
                .context("Invalid blocksize value")?;

            let device = storage_devices
                .iter()
                .find(|d| d.id() == device_id)
                .ok_or(anyhow!("Unknown device {}", device_id))?;
            let scheme = schemes
                .find(scheme_id)
                .ok_or(anyhow!("Unknown scheme {}", scheme_id))?;

            let task = WipeTask::new(
                scheme.clone(),
                verification,
                device.details().size,
                block_size,
            );
            let mut state = WipeState::default();
            let mut session = frontend.wipe_session(device_id, scheme_id, cmd.is_present("yes"));

            match System::access(device) {
                Ok(mut access) => {
                    if !task.run(&mut access, &mut state, &mut session) {
                        std::process::exit(1);
                    }
                }
                Err(err) => {
                    session.handle(&task, &state, WipeEvent::Fatal(Rc::from(err)));
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("{}", app.usage());
            std::process::exit(1)
        }
    }

    Ok(())
}
