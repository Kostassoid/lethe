extern crate clap;
use clap::{Arg, App, SubCommand, AppSettings};

#[macro_use] extern crate prettytable;
use prettytable::{Table, format};

use console::style;
use indicatif::{HumanBytes};

mod storage;
use storage::nix::*;
use storage::{StorageEnumerator, StorageRef};

mod sanitization;
use sanitization::*;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

const SCHEMES_EXPLANATION: &'static str = "Data sanitization schemes:
    gost        GOST R 50739-95, 2 passes
    dod         DOD 5220.22-M, 3 passes
    zero        Single zeroes (0x00) fill, 1 pass
    one         Single ones (0xFF) fill, 1 pass
    random      Single random fill, 1 pass
";

fn main() {

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
            .after_help(SCHEMES_EXPLANATION)
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
                .possible_values(&["zero", "one", "random", "gost", "dod"])
                .default_value("random")
                .help("Data sanitization scheme"))
            .arg(Arg::with_name("verify")
                .long("verify")
                .short("v")
                .help("Verify after completion"))
            .arg(Arg::with_name("yes")
                .long("yes")
                .short("y")
                .help("Automatically confirm"))
        )
        .get_matches();

    let enumerator = FileEnumerator::custom(
        std::env::temp_dir(), 
        |x| x.to_str().unwrap().contains("disk"), 
        |_| true
    );
    //let enumerator = FileEnumerator::system_drives();

    let schemes = SchemeRepo::default();

    match app.subcommand() {
        ("list", _) => {
            let mut t = Table::new();
            t.set_format(*format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
            t.set_titles(row!["Device ID", "Size"]);
            for x in enumerator.list().unwrap() {
                t.add_row(row![style(x.id()).bold(), HumanBytes(x.details().size)]);
            }
            t.printstd();
        },
        ("wipe", Some(cmd)) => {
            use std::io::prelude::*;

            let device_id = cmd.value_of("device").unwrap();
            let scheme_id = cmd.value_of("scheme").unwrap();

            let device = enumerator.list().unwrap().iter().find(|d| d.id() == device_id)
                .expect(&format!("Unknown device {}", device_id));
            let scheme = schemes.find(scheme_id)
                .expect(&format!("Unknown scheme {}", scheme_id));

            println!("Wiping {} using scheme {}.", style(device_id).bold(), style(scheme_id).bold());
            print!("Are you sure? (type 'yes' to confirm): ");
            std::io::stdout().flush().unwrap();

            let mut confirm = String::new();
            if std::io::stdin().read_line(&mut confirm).is_err() || confirm != "yes" {
                std::process::exit(1);    
            }
            println!("OK");
        },
        _ => {
            println!("{}", app.usage());
            std::process::exit(1)
        }
    }
}