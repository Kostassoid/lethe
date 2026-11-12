use std::fmt::Write as _;
use std::sync::mpsc::{Sender, channel};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::Result;
use indicatif::{HumanBytes, HumanDuration, MultiProgress, ProgressBar, ProgressStyle};
use prettytable::format::FormatBuilder;
use prettytable::{Table, row};

use crate::actions::{
    SectionHandlerFactory, Step, WipeEvent, WipeEventHandler, WipePlan, WipeSessionState,
};
use crate::sanitization::scheme::{Scheme, SchemeRepo, Stage};
use crate::storage::{StorageDevice, StorageRef};

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
    let passes = if stages_count > 1 { "passes" } else { "pass" };

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
    pb: Option<ProgressBar>,
    session_started: Option<Instant>,
    stage_started: Option<Instant>,
}

/// Creates a handler for wiping the given device.
pub fn wipe_session(device_id: &str, auto_confirm: bool) -> ConsoleEventHandler {
    ConsoleEventHandler {
        device_id: String::from(device_id),
        auto_confirm,
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
        if !announce_and_confirm(&self.device_id, plan, self.auto_confirm, 1) {
            std::process::exit(0);
        }
    }

    fn start_step(&mut self, plan: &WipePlan, step: usize) {
        let step_description = format!("Step {}/{}", step + 1, plan.steps.len());

        let (current_stage, at_verification) = match plan.steps[step] {
            Step::Verify(i) => (&plan.scheme.stages[i], true),
            Step::Write(i) => (&plan.scheme.stages[i], false),
        };

        let stage_description = describe_stage(current_stage);

        let pb = create_progress_bar(plan.total_bytes());

        if at_verification {
            pb.println(format!(
                "\n{step_description}: Verifying {stage_description}"
            ));
            pb.set_message("Checking");
        } else {
            pb.println(format!(
                "\n{step_description}: Performing {stage_description}"
            ));
            pb.set_message("Writing");
        }

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
    sections: usize,
) -> bool {
    let steps = plan
        .steps
        .iter()
        .map(|s| match s {
            Step::Write(_) => "fill",
            Step::Verify(_) => "verify",
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
    if sections > 1 {
        t.add_row(row!["I/O sections", sections]);
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

struct SectionResult {
    bad_blocks: u64,
}

/// Reports one section of a parallel wipe, with its own progress bar.
struct SectionEventHandler {
    section: usize,
    sections: usize,
    pb: ProgressBar,
    results: Sender<SectionResult>,
    bad_blocks: u64,
}

impl WipeEventHandler for SectionEventHandler {
    fn handle(&mut self, plan: &WipePlan, _state: &WipeSessionState, event: WipeEvent) {
        let label = format!("section {}/{}", self.section + 1, self.sections);

        match event {
            WipeEvent::Created | WipeEvent::Started => {}
            WipeEvent::StepStarted(step) => {
                let (stage, at_verification) = match plan.steps[step] {
                    Step::Verify(i) => (&plan.scheme.stages[i], true),
                    Step::Write(i) => (&plan.scheme.stages[i], false),
                };

                self.pb.set_length(plan.total_bytes());
                self.pb.set_position(0);
                self.pb.set_message(format!(
                    "{label} step {}/{} {} {}",
                    step + 1,
                    plan.steps.len(),
                    if at_verification {
                        "verifying"
                    } else {
                        "writing"
                    },
                    describe_stage(stage)
                ));
            }
            WipeEvent::Progress(position) => {
                self.pb
                    .set_position(position.saturating_sub(plan.range.start));
            }
            WipeEvent::MarkedBlockAsBad(position) => {
                self.bad_blocks += 1;
                self.pb.println(format!(
                    "[{label}] Unable to access block at {position}. Skipping."
                ));
            }
            WipeEvent::SkippedBadBlock(position) => {
                self.pb
                    .println(format!("[{label}] Skipping bad block at {position}."));
            }
            WipeEvent::StepCompleted(step) => {
                self.pb.println(format!(
                    "[{label}] Step {}/{} completed",
                    step + 1,
                    plan.steps.len()
                ));
            }
            WipeEvent::StepFailed(step, err) => {
                self.pb.println(format!(
                    "[{label}] FAILED at step {}/{}: {err}",
                    step + 1,
                    plan.steps.len()
                ));
            }
            WipeEvent::Retrying(step, position) => {
                self.pb.println(format!(
                    "[{label}] Retrying step {}/{} from position {position} in {RETRY_BACKOFF_SECONDS} seconds.",
                    step + 1,
                    plan.steps.len()
                ));
                sleep(Duration::from_secs(u64::from(RETRY_BACKOFF_SECONDS)));
            }
            WipeEvent::Completed => {
                self.pb.finish_with_message(format!("{label} done"));
                let _ = self.results.send(SectionResult {
                    bad_blocks: self.bad_blocks,
                });
            }
            WipeEvent::Failed(err) => {
                self.pb
                    .finish_with_message(format!("{label} failed: {err}"));
                let _ = self.results.send(SectionResult {
                    bad_blocks: self.bad_blocks,
                });
            }
        }
    }
}

/// Runs a wipe as several independent sections, each with its own storage handle and
/// progress bar. Verification coverage is still computed over the whole planned area.
pub fn run_parallel_wipe(
    device: &StorageRef,
    plan: WipePlan,
    retries: u32,
    queue_depth: usize,
    auto_confirm: bool,
) -> Result<()> {
    let mut storages = device.access_many(queue_depth)?;

    // Platforms may provide fewer handles than requested (for example, a partition
    // target on Windows), and a small area may not have enough blocks for every section.
    let sections = plan.split(storages.len());
    storages.truncate(sections.len());

    if sections.len() <= 1 {
        let storage = storages
            .pop()
            .ok_or_else(|| anyhow::anyhow!("No storage handle was opened"))?;
        let session = wipe_session(&device.id, auto_confirm);
        return plan.execute(storage, Box::new(session), retries);
    }

    if !announce_and_confirm(&device.id, &plan, auto_confirm, sections.len()) {
        return Ok(());
    }

    let section_count = sections.len();

    let (results_tx, results_rx) = channel();
    let multi = MultiProgress::new();
    let mut handlers: Vec<SectionHandlerFactory> = Vec::with_capacity(section_count);

    for (index, section) in sections.iter().enumerate() {
        let pb = multi.add(create_progress_bar(section.total_bytes()));
        pb.set_message(format!("section {}/{}", index + 1, section_count));

        let results = results_tx.clone();
        handlers.push(Box::new(move || {
            Box::new(SectionEventHandler {
                section: index,
                sections: section_count,
                pb,
                results,
                bad_blocks: 0,
            })
        }));
    }

    drop(results_tx);

    let started = Instant::now();
    let result = WipePlan::execute_sections(sections, storages, handlers, retries);
    let outcomes: Vec<SectionResult> = results_rx.try_iter().collect();

    if result.is_ok() {
        let bad_blocks = outcomes.iter().map(|outcome| outcome.bad_blocks).sum();
        print_summary(&plan, bad_blocks, Some(started.elapsed()));
    }

    result
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
