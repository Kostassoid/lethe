use std::io::ErrorKind;
use std::time::Instant;

use indicatif::{HumanBytes, HumanDuration, ProgressBar, ProgressStyle};

use crate::actions::{EntropyScan, WipeEvent, WipeEventReceiver, WipeState, WipeTask};
use crate::sanitization::{Scheme, SchemeRepo};
use crate::stage::Stage;
use prettytable::format::FormatBuilder;
use prettytable::Table;
use std::thread::sleep;

const RETRY_BACKOFF_SECONDS: u32 = 3;

pub struct ConsoleFrontend {}

impl ConsoleFrontend {
    pub fn new() -> Self {
        ConsoleFrontend {}
    }

    pub fn wipe_session(self, device_id: &str, auto_confirm: bool) -> ConsoleWipeSession {
        ConsoleWipeSession {
            device_id: String::from(device_id),
            auto_confirm,
            pb: None,
            session_started: None,
            stage_started: None,
        }
    }

    pub fn explain_schemes(schemes: &SchemeRepo) -> String {
        let mut t = Table::new();
        let indent_table_format = FormatBuilder::new().padding(4, 1).build();
        t.set_format(indent_table_format);
        for (k, v) in schemes.all().iter() {
            t.add_row(row![k, Self::describe_scheme(v)]);
        }

        format!("Data sanitization schemes:\n{}", t)
    }

    pub fn sampling_progress(size: u64) -> ProgressBar {
        let progress = create_progress_bar(size);
        progress.set_message("Sampling");
        progress
    }

    pub fn print_entropy_scan(scan: &EntropyScan) {
        println!(
            "Sampled {} blocks ({} bytes each; {} sample(s) of {} bytes per block).",
            scan.blocks_scanned, scan.block_size, scan.samples_per_block, scan.sample_size
        );
        println!("Starting offset: {} bytes.", scan.offset);
        println!(
            "Shannon entropy by offset (bits/byte; mid-entropy candidates: {:.1} to {:.1}):",
            crate::actions::MID_ENTROPY_MIN,
            crate::actions::MID_ENTROPY_MAX
        );

        for level in (1..=8).rev() {
            print!("{:>2} |", level);
            for entropy in &scan.graph {
                match entropy {
                    Some(value) if *value >= level as f64 => print!("#"),
                    Some(_) | None => print!(" "),
                }
            }
            println!();
        }
        println!(" 0 +{}", "-".repeat(scan.graph.len()));
        println!(
            "     start{}end",
            " ".repeat(scan.graph.len().saturating_sub(8))
        );

        println!(
            "{} of {} sampled blocks fall in the mid-entropy band.",
            scan.candidate_blocks, scan.blocks_scanned
        );
        if scan.region_count == 0 {
            println!("No mid-entropy regions found.");
        } else {
            println!(
                "Candidate regions (showing up to {}; start..end byte offsets):",
                scan.regions.len()
            );
            for region in &scan.regions {
                println!(
                    "  {}..{} ({} bytes)",
                    region.start,
                    region.end,
                    HumanBytes(region.end - region.start)
                );
            }
            if scan.region_count > scan.regions.len() as u64 {
                println!(
                    "  ... {} additional regions omitted.",
                    scan.region_count - scan.regions.len() as u64
                );
            }
        }
    }

    fn describe_scheme(scheme: &Scheme) -> String {
        let mut s = String::new();

        let stages_count = scheme.stages.len();
        let passes = if stages_count != 1 { "passes" } else { "pass" };

        s.push_str(&format!(
            "{}, {} {}\n",
            scheme.description, stages_count, passes
        ));

        for v in &scheme.stages {
            s.push_str(&format!("- {}\n", v));
        }

        s
    }
}

pub struct ConsoleWipeSession {
    device_id: String,
    auto_confirm: bool,
    pb: Option<ProgressBar>,
    session_started: Option<Instant>,
    stage_started: Option<Instant>,
}

impl WipeEventReceiver for ConsoleWipeSession {
    fn handle(&mut self, task: &WipeTask, state: &WipeState, event: WipeEvent) -> () {
        match event {
            WipeEvent::Created => {
                let mut t = Table::new();
                let indent_table_format = FormatBuilder::new().padding(4, 1).build();
                t.set_format(indent_table_format);
                t.add_row(row!["Device", self.device_id]);
                t.add_row(row![
                    "Size",
                    format!(
                        "{} ({} bytes)",
                        HumanBytes(task.total_size),
                        task.total_size
                    )
                ]);
                t.add_row(row![
                    "Scheme",
                    ConsoleFrontend::describe_scheme(&task.scheme)
                ]);
                t.add_row(row!["Block size", HumanBytes(task.block_size as u64)]);
                t.add_row(row![
                    "Starting offset",
                    format!("{} ({} bytes)", HumanBytes(task.offset), task.offset)
                ]);
                t.add_row(row!["Verification", task.verify]);
                print!("Wiping:\n{}", t);

                if !self.auto_confirm
                    && !ask_for_confirmation("Are you sure? (type 'yes' to confirm): ")
                {
                    println!("Aborted.");
                    std::process::exit(0);
                }
            }
            WipeEvent::Started => {
                self.session_started = Some(Instant::now());
            }
            WipeEvent::StageStarted => {
                let stage_num = format!("Stage {}/{}", state.stage + 1, task.scheme.stages.len());
                let stage = &task.scheme.stages[state.stage];

                let stage_description = match stage {
                    Stage::Fill { value } => format!("Value Fill ({:02x})", value),
                    Stage::Random { seed: _seed } => String::from("Random Fill"),
                };

                let pb = create_progress_bar(task.total_size);

                if !state.at_verification {
                    pb.println(format!("\n{}: Performing {}", stage_num, stage_description));
                } else {
                    pb.println(format!("\n{}: Verifying {}", stage_num, stage_description));
                }

                if !state.at_verification {
                    pb.set_message("Writing");
                } else {
                    pb.set_message("Checking");
                }

                self.pb = Some(pb);
                self.stage_started = Some(Instant::now());
            }
            WipeEvent::Progress(position) => {
                if let Some(pb) = &self.pb {
                    pb.set_position(position);
                }
            }
            WipeEvent::MarkedBlockAsBad(block) => {
                if let Some(pb) = &self.pb {
                    pb.println(format!("Unable to access block at {}. Skipping.", block));
                }
            }
            WipeEvent::StageCompleted(result) => {
                if let Some(pb) = &self.pb {
                    match result {
                        None => {
                            if let Some(s) = self.stage_started {
                                let elapsed = HumanDuration(s.elapsed());
                                pb.println(format!("✔ Completed in {}", elapsed));
                            } else {
                                pb.println("✔ Completed");
                            }
                        }
                        Some(err) => {
                            pb.println(format!("❌ FAILED! {:#}", err));
                        }
                    }
                    pb.finish_and_clear();
                }
            }
            WipeEvent::Retrying => {
                eprintln!(
                    "Retrying previous stage at {} in {} seconds.",
                    state.position, RETRY_BACKOFF_SECONDS
                );
                sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECONDS as u64));
            }
            WipeEvent::Completed(result) => match result {
                None => {
                    if let Some(s) = self.session_started {
                        let elapsed = HumanDuration(s.elapsed());
                        println!("✔ Total time: {}", elapsed);
                    }
                    let total_blocks = task.total_size / task.block_size as u64;
                    let wiped_blocks = (task.total_size - task.offset) / task.block_size as u64;
                    let bad_blocks = state.bad_blocks.borrow_mut().total_marked();

                    let mut t = Table::new();
                    let indent_table_format = FormatBuilder::new().padding(4, 1).build();
                    t.set_format(indent_table_format);
                    t.add_row(row!["Total device size", HumanBytes(task.total_size)]);
                    t.add_row(row!["Total device blocks", total_blocks]);
                    match task.selected_block_count() {
                        Some(blocks) => {
                            t.add_row(row!["Blocks selected by entropy", blocks]);
                        }
                        None => {
                            t.add_row(row!["Total blocks wiped", wiped_blocks]);
                        }
                    }
                    t.add_row(row![
                        "Skipped blocks",
                        format!(
                            "{} ({}%)",
                            bad_blocks,
                            bad_blocks * 100 / total_blocks as u32
                        )
                    ]);

                    print!("{}", t);
                }
                Some(e) => {
                    eprintln!("❌ Unexpected error: {:#}", e);

                    if let Some(ioe) = e.downcast_ref::<std::io::Error>() {
                        if ioe.kind() == ErrorKind::Other && ioe.raw_os_error() == Some(16) {
                            eprintln!("Make sure the drive is not mounted.")
                        }
                    };
                }
            },
            WipeEvent::Fatal(err) => {
                eprintln!("❌ Fatal: {:#}", err);
            }
        }
    }
}

pub fn ask_for_confirmation(prompt: &str) -> bool {
    use std::io::prelude::*;

    print!("{}", prompt);
    std::io::stdout().flush().unwrap();

    let mut confirm = String::new();
    std::io::stdin().read_line(&mut confirm).is_ok() && confirm.trim() == "yes"
}

fn create_progress_bar(size: u64) -> ProgressBar {
    let pb = ProgressBar::new(size);

    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.red/black} {bytes:>7}/{total_bytes:7} ({eta} left) {msg}")
        .unwrap() // panic is OK here, although some test coverage would be nice
        .progress_chars("█▉▊▋▌▍▎▏  "));

    pb
}
