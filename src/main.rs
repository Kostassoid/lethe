extern crate clap;
use clap::{Arg, App, SubCommand, AppSettings};

mod storage;
use storage::nix::*;
use storage::{StorageEnumerator, StorageRef};

mod sanitization;
use sanitization::SanitizationStage;

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

    match app.subcommand() {
        ("list", _) => 
            for x in enumerator.iterate().unwrap() {
                println!("-- {} ({:?})", x.id(), x.details());
            },
        ("wipe", Some(cmd)) => {
                let device = cmd.value_of("device").unwrap();
                let scheme = cmd.value_of("scheme").unwrap();
                println!("Wiping {} using scheme {}", device, scheme)
            },
        _ => 
            println!("{}", app.usage())
    }
}