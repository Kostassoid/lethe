extern crate clap;
use clap::{Arg, App, SubCommand, AppSettings};

#[macro_use] extern crate prettytable;
use prettytable::{Table, format};
use format::FormatBuilder;

use ::console::style;
use indicatif::{HumanBytes};

mod storage;
use storage::*;

mod sanitization;
use sanitization::*;

mod wiper;
use wiper::*;

mod ui;
use ui::*;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn main() {

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
            t.add_row(row![k, format!("{}, {} {}", v.description, stages_count, passes)]);
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
        .subcommand(SubCommand::with_name("list")
            .about("list available storage devices")
        )
        .subcommand(SubCommand::with_name("wipe")
            .about("Wipe storage device")
            .after_help(schemes_explanation.as_str()) 
            .arg(Arg::with_name("device")
                .long("device")
                .short("d")
                .required(true)
                .takes_value(true)
                .index(1)
                .help("Storage device ID"))
            .arg(Arg::with_name("scheme")
                .long("scheme")
                .short("s")
                .takes_value(true)
                .possible_values(&scheme_keys)
                .default_value("random2")
                .help("Data sanitization scheme"))
            .arg(Arg::with_name("verify")
                .long("verify")
                .short("v")
                .takes_value(true)
                .possible_values(&["no", "last", "all"])
                .default_value("last")
                .help("Verify after completion"))
            .arg(Arg::with_name("blocksize")
                .long("blocksize")
                .short("bs")
                .takes_value(true)
                .help("Block size override (bytes)"))
            .arg(Arg::with_name("yes")
                .long("yes")
                .short("y")
                .help("Automatically confirm"))
        )
        .get_matches();

    let storage_devices = System::get_storage_devices().unwrap(); //todo: handle errors

    let mut frontend = cli::ConsoleFrontend::new();

    match app.subcommand() {
        ("list", _) => {
            let mut t = Table::new();
            t.set_format(*format::consts::FORMAT_CLEAN);
            t.set_titles(row!["Device ID", "Size", "Block Size"]);
            for x in storage_devices {
                t.add_row(row![style(x.id()).bold(), HumanBytes(x.details().size), HumanBytes(x.details().block_size as u64)]);
            }
            t.printstd();
        },
        ("wipe", Some(cmd)) => {
            let device_id = cmd.value_of("device").unwrap();
            let scheme_id = cmd.value_of("scheme").unwrap();
            let verification = match cmd.value_of("verify").unwrap() {
                "no" => Verify::No,
                "last" => Verify::Last,
                "all" => Verify::All,
                _ => Verify::Last
            };
            let block_size_override = cmd.value_of("blocksize")
                .map(|bs| parse_block_size(bs)
                    .unwrap_or_else(|err| {
                        eprintln!("Invalid blocksize value. {}", err);
                        std::process::exit(1);
                    }));

            let device = storage_devices.iter().find(|d| d.id() == device_id)
                .unwrap_or_else(|| {
                    eprintln!("Unknown device {}", device_id);
                    std::process::exit(1);
                });
            let scheme = schemes.find(scheme_id)
                .unwrap_or_else(|| {
                    eprintln!("Unknown scheme {}", scheme_id);
                    std::process::exit(1);
                });

            let block_size = block_size_override.unwrap_or(device.details().block_size);

            println!("Wiping {} using scheme {} and block size {}.", 
                style(device_id).bold(), 
                style(scheme_id).bold(),
                style(HumanBytes(block_size as u64)).bold()
            );
            if !cmd.is_present("yes") && !ask_for_confirmation() {
                println!("Aborted.");
                std::process::exit(0);
            } else {
                let task = WiperTask::new(scheme.clone(), verification, device.details().size, block_size);
                let mut state = WiperState::default();
                let mut access = *device.access().unwrap(); //todo: handle errors
                
                if !task.run(&mut access, &mut state, &mut frontend) {
                    std::process::exit(1);
                }
            }
        },
        _ => {
            println!("{}", app.usage());
            std::process::exit(1)
        }
    }
}

fn parse_block_size(s: &str) -> Result<usize, std::num::ParseIntError> {
    s.parse::<usize>()
}

fn ask_for_confirmation() -> bool {
    use std::io::prelude::*;

    print!("Are you sure? (type 'yes' to confirm): ");
    std::io::stdout().flush().unwrap();

    let mut confirm = String::new();
    std::io::stdin().read_line(&mut confirm).is_ok() && confirm.trim() == "yes"
}
