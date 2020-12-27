use std::io::ErrorKind;
use std::time::Instant;

use indicatif::{HumanBytes, HumanDuration, ProgressBar, ProgressStyle};

use crate::actions::{WipeEvent, WipeEventReceiver, WipeState, WipeTask};
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
            WipeEvent::Started => {
                let mut t = Table::new();
                let indent_table_format = FormatBuilder::new().padding(4, 1).build();
                t.set_format(indent_table_format);
                t.add_row(row!["Device", self.device_id]);
                t.add_row(row!["Size", HumanBytes(task.total_size)]);
                t.add_row(row!["Scheme", super::describe_scheme(&task.scheme)]);
                t.add_row(row!["Block size", HumanBytes(task.block_size as u64)]);
                t.add_row(row!["Verification", task.verify]);
                print!("Wiping:\n{}", t);

                if !self.auto_confirm && !ask_for_confirmation() {
                    println!("Aborted.");
                    std::process::exit(0);
                }
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
            WipeEvent::MarkBlockAsBad(block) => {
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
                            pb.println(format!("❌ FAILED! {}", err));
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
                    let bad_blocks = state.bad_blocks.borrow_mut().total_marked();

                    let mut t = Table::new();
                    let indent_table_format = FormatBuilder::new().padding(4, 1).build();
                    t.set_format(indent_table_format);
                    t.add_row(row!["Total device size", HumanBytes(task.total_size)]);
                    t.add_row(row!["Total blocks", total_blocks]);
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
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {bytes:>7}/{total_bytes:7} ({eta} left) {msg}")
        .progress_chars("█▉▊▋▌▍▎▏  "));

    pb
}
