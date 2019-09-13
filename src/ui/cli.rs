use std::io::ErrorKind;

use indicatif::{ProgressBar, ProgressStyle};

use crate::wiper::{WiperEventReceiver, WiperEvent, WiperState, WiperTask};
use crate::stage::Stage;

pub struct ConsoleFrontend {
    pb: Option<ProgressBar>
}

impl ConsoleFrontend {
    pub fn new() -> Self {
        ConsoleFrontend { pb: None }
    }
}

impl WiperEventReceiver for ConsoleFrontend {
    fn handle(&mut self, task: &WiperTask, state: &WiperState, event: WiperEvent) -> () {
        match event {
            WiperEvent::StageStarted => {
                let stage_num = format!("Stage {}/{}", state.stage + 1, task.scheme.stages.len());
                let stage = &task.scheme.stages[state.stage];
                
                let stage_description = match stage {
                    Stage::Fill { value } => format!("Value Fill ({:02x})", value),
                    Stage::Random { seed: _seed } => String::from("Random Fill")
                };

                if !state.at_verification {
                    println!("\n{}: Performing {}", stage_num, stage_description);
                } else {
                    println!("\n{}: Verifying {}", stage_num, stage_description);
                }

                let pb = create_progress_bar(task.total_size);

                if !state.at_verification {
                    pb.set_message("Writing");
                } else {
                    pb.set_message("Checking");
                }

                self.pb = Some(pb);
            },
            WiperEvent::Progress(position) => { 
                if let Some(pb) = &self.pb {
                    pb.set_position(position);
                }
            },
            WiperEvent::StageCompleted(result) => {
                if let Some(pb) = &self.pb {
                    match result {
                        Ok(_) => pb.finish_with_message("Done"),
                        Err(err) => { 
                            pb.finish_with_message("FAILED!");
                            eprintln!("Error: {}", err);
                        },
                    }
                }
            },
            WiperEvent::Retrying => {
                eprintln!("Retrying previous stage at {}.", state.position);
            },
            WiperEvent::Aborted => {
                eprintln!("Aborted.");
            },
            WiperEvent::Completed(result) => {
                match result {
                    Ok(_) => println!("Done."),
                    Err(e) => {
                        eprintln!("Unexpected error: {}", e);
                        match (e.kind(), e.raw_os_error()) {
                            (ErrorKind::Other, Some(16)) => 
                                eprintln!("Make sure the drive is not mounted."),
                            _ => ()
                        };
                    }
                }

            }
        }
    }
}

fn create_progress_bar(size: u64) -> ProgressBar {
    let pb = ProgressBar::new(size);

    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {bytes:>7}/{total_bytes:7} ({eta} left) {msg}")
        .progress_chars("█▉▊▋▌▍▎▏  "));

    pb
}