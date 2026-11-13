use std::fmt::Write as _;
use std::thread::sleep;
use std::time::{Duration, Instant};

use indicatif::{HumanBytes, HumanDuration, ProgressBar, ProgressStyle};
use prettytable::format::FormatBuilder;
use prettytable::{Table, row};

use crate::actions::{Step, WipeEvent, WipeEventHandler, WipePlan, WipeSessionState};
use crate::sanitization::scheme::{Scheme, SchemeRepo, Stage};

const RETRY_BACKOFF_SECONDS: u32 = 3;

/// Renders the scheme catalogue as a help-text table.
pub fn explain_schemes(schemes: &SchemeRepo) -> String {
    let mut t = Table::new();
    t.set_format(indented_table_format());
    for (k, v) in schemes.all() {
        t.add_row(row![k, describe_scheme(v)]);
    }
    format!("Data sanitization schemes:\n{t}")
}

fn describe_scheme(scheme: &Scheme) -> String {
    let stages_count = scheme.stages.len();
    let passes = if stages_count == 1 { "pass" } else { "passes" };

    let mut description = format!("{}, {stages_count} {passes}\n", scheme.description);
    for stage in &scheme.stages {
        let _ = writeln!(description, "- {stage}");
    }

    description
}

/// An event handler that reports wipe progress to the console.
pub struct ConsoleEventHandler {
    device_id: String,
    auto_confirm: bool,
    queue_depth: usize,
    pb: Option<ProgressBar>,
    session_started: Option<Instant>,
    stage_started: Option<Instant>,
}

/// Creates a handler for wiping the given device.
pub fn wipe_session(
    device_id: &str,
    auto_confirm: bool,
    queue_depth: usize,
) -> ConsoleEventHandler {
    ConsoleEventHandler {
        device_id: String::from(device_id),
        auto_confirm,
        queue_depth,
        pb: None,
        session_started: None,
        stage_started: None,
    }
}

impl WipeEventHandler for ConsoleEventHandler {
    fn handle(&mut self, plan: &WipePlan, state: &WipeSessionState, event: WipeEvent) {
        match event {
            WipeEvent::Created => self.announce_plan(plan),
            WipeEvent::Started => {
                self.session_started = Some(Instant::now());
            }
            WipeEvent::StepStarted(step) => self.start_step(plan, step),
            WipeEvent::Progress(position) => {
                if let Some(pb) = &self.pb {
                    pb.set_position(position.saturating_sub(plan.range.start));
                }
            }
            WipeEvent::SkippedBadBlock(position) => {
                if let Some(pb) = &self.pb {
                    pb.println(format!("Skipping bad block at {position}."));
                }
            }
            WipeEvent::MarkedBlockAsBad(position) => {
                if let Some(pb) = &self.pb {
                    pb.println(format!("Unable to access block at {position}. Skipping."));
                }
            }
            WipeEvent::StepCompleted(step) => {
                if let Some(pb) = self.pb.take() {
                    if let Some(stage_started) = self.stage_started.take() {
                        let elapsed = HumanDuration(stage_started.elapsed());
                        pb.println(format!(
                            "✔ Step {}/{} completed in {elapsed}",
                            step + 1,
                            plan.steps.len()
                        ));
                    } else {
                        pb.println(format!(
                            "✔ Step {}/{} completed",
                            step + 1,
                            plan.steps.len()
                        ));
                    }
                    pb.finish_and_clear();
                }
            }
            WipeEvent::StepFailed(step, err) => {
                if let Some(pb) = self.pb.take() {
                    pb.println(format!(
                        "❌ FAILED at step {}/{}: {err:#}",
                        step + 1,
                        plan.steps.len()
                    ));
                    pb.finish_and_clear();
                }
            }
            WipeEvent::Retrying(step, position) => {
                eprintln!(
                    "Retrying step {}/{} from position {position} in {RETRY_BACKOFF_SECONDS} seconds.",
                    step + 1,
                    plan.steps.len()
                );
                sleep(Duration::from_secs(u64::from(RETRY_BACKOFF_SECONDS)));
            }
            WipeEvent::Completed => self.print_summary(plan, state),
            WipeEvent::Failed(err) => {
                eprintln!("❌ Unexpected error: {err:#}");
            }
        }
    }
}

impl ConsoleEventHandler {
    fn announce_plan(&mut self, plan: &WipePlan) {
        if !announce_and_confirm(&self.device_id, plan, self.auto_confirm, self.queue_depth) {
            std::process::exit(0);
        }
    }

    fn start_step(&mut self, plan: &WipePlan, step: usize) {
        let step_description = format!("Step {}/{}", step + 1, plan.steps.len());

        let pb = match plan.steps[step] {
            Step::HardwareErase(method) => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner} {elapsed_precise} {msg}")
                        .unwrap(), // panic is OK here, although some test coverage would be nice
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.println(format!("\n{step_description}: Hardware erase ({method})"));
                pb.set_message(method.to_string());
                pb
            }
            Step::Verify(i) => {
                let stage_description = describe_stage(&plan.scheme.stages[i]);
                let pb = create_progress_bar(plan.total_bytes());
                pb.println(format!(
                    "\n{step_description}: Verifying {stage_description}"
                ));
                pb.set_message("Checking");
                pb
            }
            Step::Write(i) => {
                let stage_description = describe_stage(&plan.scheme.stages[i]);
                let pb = create_progress_bar(plan.total_bytes());
                pb.println(format!(
                    "\n{step_description}: Performing {stage_description}"
                ));
                pb.set_message("Writing");
                pb
            }
        };

        self.pb = Some(pb);
        self.stage_started = Some(Instant::now());
    }

    fn print_summary(&mut self, plan: &WipePlan, state: &WipeSessionState) {
        print_summary(
            plan,
            state.bad_blocks.len(),
            self.session_started.map(|started| started.elapsed()),
        );
    }
}

/// Prints the wipe plan and asks for confirmation. Returns `false` when the user aborts.
pub fn announce_and_confirm(
    device_id: &str,
    plan: &WipePlan,
    auto_confirm: bool,
    queue_depth: usize,
) -> bool {
    let steps = plan
        .steps
        .iter()
        .map(|s| match s {
            Step::Write(_) => "fill".to_string(),
            Step::Verify(_) => "verify".to_string(),
            Step::HardwareErase(method) => format!("erase ({method})"),
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut t = Table::new();
    t.set_format(indented_table_format());
    t.add_row(row!["Device", device_id]);
    t.add_row(row![
        "Size",
        format!("{} ({} bytes)", HumanBytes(plan.range.end), plan.range.end)
    ]);
    t.add_row(row!["Steps", steps]);
    t.add_row(row!["Block size", HumanBytes(plan.block_size as u64)]);
    t.add_row(row![
        "Starting offset",
        format!(
            "{} ({} bytes)",
            HumanBytes(plan.range.start),
            plan.range.start
        )
    ]);
    t.add_row(row![
        "Total area size",
        format!(
            "{} ({} bytes)",
            HumanBytes(plan.total_bytes()),
            plan.total_bytes()
        )
    ]);
    t.add_row(row!["Verification", &plan.verification]);
    if queue_depth > 1 {
        t.add_row(row!["I/O queue depth", queue_depth]);
    }
    print!("Wiping:\n{t}");

    if !auto_confirm && !ask_for_confirmation() {
        println!("Aborted.");
        return false;
    }

    true
}

fn describe_stage(stage: &Stage) -> String {
    match stage {
        Stage::Fill { value } => format!("Value Fill ({value:02x})"),
        Stage::Random => "Random Fill".to_string(),
        Stage::Incremental { .. } => "Incremental fill (per block)".to_string(),
    }
}

fn print_summary(plan: &WipePlan, bad_blocks: u64, elapsed: Option<Duration>) {
    if let Some(elapsed) = elapsed {
        println!("✔ Total time: {}", HumanDuration(elapsed));
    }

    let total_blocks = plan.total_blocks();
    let skipped_percent = match total_blocks {
        0 => 0,
        total => bad_blocks * 100 / total,
    };

    let mut t = Table::new();
    t.set_format(indented_table_format());
    t.add_row(row![
        "Total covered area",
        format!(
            "{} - {} ({})",
            HumanBytes(plan.range.start),
            HumanBytes(plan.range.end),
            HumanBytes(plan.total_bytes())
        )
    ]);
    t.add_row(row!["Total blocks", total_blocks]);
    t.add_row(row![
        "Skipped blocks",
        format!("{bad_blocks} ({skipped_percent}%)")
    ]);

    print!("{t}");
}

fn indented_table_format() -> prettytable::format::TableFormat {
    FormatBuilder::new().padding(4, 1).build()
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
        .template("[{elapsed_precise}] {bar:40.red/black} {bytes:>7}/{total_bytes:7} ({eta} left) {msg}")
        .unwrap() // panic is OK here, although some test coverage would be nice
        .progress_chars("█▉▊▋▌▍▎▏  "));

    pb
}
