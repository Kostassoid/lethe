use std::io::{Error, ErrorKind};

extern crate clap;
use clap::{Arg, App, SubCommand, AppSettings};

#[macro_use] extern crate prettytable;
use prettytable::{Table, format};
use format::FormatBuilder;

use console::style;
use indicatif::{HumanBytes};
use streaming_iterator::StreamingIterator;

mod storage;
use storage::nix::*;
use storage::*;

mod sanitization;
use sanitization::*;
use sanitization::stage::Stage;

use indicatif::{ProgressBar, ProgressStyle};

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

enum Verify {
    No,
    Last,
    All
}

fn main() {

    let indent_table_format = FormatBuilder::new().padding(4, 1).build();

    let schemes = SchemeRepo::default();
    let scheme_keys: Vec<_> = schemes.all().keys().cloned().collect();

    let schemes_explanation = { 
        let mut t = Table::new();
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
        ("list", _) => {
            let mut t = Table::new();
            t.set_format(*format::consts::FORMAT_CLEAN);
            t.set_titles(row!["Device ID", "Size"]);
            for x in enumerator.list().unwrap() {
                t.add_row(row![style(x.id()).bold(), HumanBytes(x.details().size)]);
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

            let device_list = enumerator.list().unwrap();
            let device = device_list.iter().find(|d| d.id() == device_id)
                .expect(&format!("Unknown device {}", device_id));
            let scheme = schemes.find(scheme_id)
                .expect(&format!("Unknown scheme {}", scheme_id));

            println!("Wiping {} using scheme {}.", style(device_id).bold(), style(scheme_id).bold());
            if !cmd.is_present("yes") && !ask_for_confirmation() {
                println!("Aborted.");
                std::process::exit(1);    
            } else {
                wipe(device, scheme, verification).unwrap();
            }
        },
        _ => {
            println!("{}", app.usage());
            std::process::exit(1)
        }
    }
}

fn wipe<A: StorageRef>(device: &A, scheme: &Scheme, verification: Verify) -> IoResult<()> {
    let stages = &scheme.stages;
    let mut access = device.access()?;
    for (i, stage) in stages.iter().enumerate() {

        let stage_num = format!("Stage {}/{}", i + 1, scheme.stages.len());
        let stage_description = match stage {
            Stage::Fill { value } => format!("Value Fill ({:02x})", value),
            Stage::Random { seed: _seed } => String::from("Random Fill")
        };

        let have_to_verify = match verification {
            Verify::No => false,
            Verify::Last if i + 1 == scheme.stages.len() => true,
            Verify::All => true,
            _ => false
        };

        loop {
            println!("\n{}: Performing {}", stage_num, stage_description);
            fill(&mut *access, stage, device.details().size, device.details().block_size)?;

            if !have_to_verify {
                break;
            }

            println!("\n{}: Verifying {}", stage_num, stage_description);
            
            if let Err(err) = verify(&mut *access, stage, device.details().size, device.details().block_size) {
                println!("Error: {}\nRetrying previous stage.", err);
            } else {
                break;
            }
        }

    }
    Ok(())
}

fn fill<A: StorageAccess>(access: &mut A, stage: &Stage, total_size: u64, block_size: usize) -> IoResult<()> {
        let pb = create_progress_bar(total_size);
        pb.set_message("Writing");

        access.seek(0u64)?;

        let mut stream = stage.stream(
            total_size, 
            block_size);

        while let Some(chunk) = stream.next() {
            access.write(chunk)?;
            pb.inc(chunk.len() as u64);
            std::thread::sleep_ms(500);
        };

        access.flush()?;
        pb.finish_with_message("Done");

        Ok(())
}

fn verify<A: StorageAccess>(access: &mut A, stage: &Stage, total_size: u64, block_size: usize) -> IoResult<()> {
        let pb = create_progress_bar(total_size);
        pb.set_message("Checking");

        access.seek(0u64)?;

        let mut stream = stage.stream(
            total_size, 
            block_size);

        let mut buf: Vec<u8> = vec![0; block_size];

        while let Some(chunk) = stream.next() {
            let b = &mut buf[..chunk.len()];
            access.read(b)?;
            if b != chunk {
                pb.finish_with_message("FAILED!");
                return Err(Error::new(ErrorKind::InvalidData, "Verification failed!"));
            }

            pb.inc(chunk.len() as u64);
            std::thread::sleep_ms(500);
        }

        pb.finish_with_message("Done");

        Ok(())
}

fn ask_for_confirmation() -> bool {
    use std::io::prelude::*;

    print!("Are you sure? (type 'yes' to confirm): ");
    std::io::stdout().flush().unwrap();

    let mut confirm = String::new();
    std::io::stdin().read_line(&mut confirm).is_ok() && confirm.trim() == "yes"
}

fn create_progress_bar(size: u64) -> ProgressBar {
    let pb = ProgressBar::new(size);

    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {bytes:>7}/{total_bytes:7} ({eta}) {msg}")
        .progress_chars("#>-"));

    pb
}