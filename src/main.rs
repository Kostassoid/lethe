extern crate clap;
use clap::{Arg, App, SubCommand};

mod storage;
use storage::nix::*;
use crate::storage::{StorageEnumerator, StorageRef};

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn main() {

    let matches = App::new("Lethe")
        .version(VERSION)
        .author("https://github.com/Kostassoid/lethe")
        .about("Secure disk wipe")
        .subcommand(SubCommand::with_name("list")
            .about("list available storage devices")
        )
        .subcommand(SubCommand::with_name("wipe")
            .about("wipe storage device")
            .arg(Arg::with_name("debug")
                .short("d")
                .help("print debug information verbosely"))
        )
        .get_matches();

    // Gets a value for config if supplied by user, or defaults to "default.conf"
//    let config = matches.value_of("config").unwrap_or("default.conf");
//    println!("Value for config: {}", config);

    // Calling .unwrap() is safe here because "INPUT" is required (if "INPUT" wasn't
    // required we could have used an 'if let' to conditionally get the value)
//    println!("Using input file: {}", matches.value_of("INPUT").unwrap());

    // Vary the output based on how many times the user used the "verbose" flag
    // (i.e. 'myprog -v -v -v' or 'myprog -vvv' vs 'myprog -v'
//    match matches.occurrences_of("v") {
//        0 => println!("No verbose info"),
//        1 => println!("Some verbose info"),
//        2 => println!("Tons of verbose info"),
//        3 | _ => println!("Don't be crazy"),
//    }

    // You can handle information about subcommands by requesting their matches by name
    // (as below), requesting just the name used, or both at the same time
    if let Some(matches) = matches.subcommand_matches("list") {
        //let enumerator = FileEnumerator::new(std::env::temp_dir());
        let enumerator = FileEnumerator::system_drives();
        for x in enumerator.iterate().unwrap() {
            println!("-- {} ({:?})", x.id(), x.details());
        }
    }
}
