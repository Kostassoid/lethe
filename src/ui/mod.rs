use crate::sanitization::{Scheme, SchemeRepo};
use prettytable::format::FormatBuilder;
use prettytable::Table;

pub mod args;
pub mod cli;
pub mod idshortcuts;

pub fn explain_schemes(schemes: &SchemeRepo) -> String {
    let mut t = Table::new();
    let indent_table_format = FormatBuilder::new().padding(4, 1).build();
    t.set_format(indent_table_format);
    for (k, v) in schemes.all().iter() {
        t.add_row(row![k, describe_scheme(v)]);
    }
    format!("Data sanitization schemes:\n{}", t)
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
