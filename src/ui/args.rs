use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command, value_parser};
use regex::Regex;

use crate::sanitization::SchemeRepo;

pub use crate::storage::HardwareEraseMethod;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Queue depth requested on the command line, before drive-based auto detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueDepth {
    Auto,
    Fixed(usize),
}

impl std::str::FromStr for QueueDepth {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }

        let depth: u32 = value
            .parse()
            .map_err(|_| "expected 'auto' or an integer".to_string())?;

        if !(1..=256).contains(&depth) {
            return Err("queue depth must be between 1 and 256".to_string());
        }

        Ok(Self::Fixed(
            usize::try_from(depth).expect("u32 fits into usize"),
        ))
    }
}

/// Builds the command-line interface of the application.
pub fn build_command(schemes: &SchemeRepo) -> Command {
    let scheme_keys: Vec<_> = schemes.all().keys().copied().collect();
    let schemes_explanation = crate::ui::cli::explain_schemes(schemes);

    Command::new("Lethe")
        .version(VERSION)
        .author("https://github.com/Kostassoid/lethe")
        .about("Secure disk wipe")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("list available storage devices"))
        .subcommand(
            Command::new("wipe")
                .about("Wipe storage device")
                .after_help(schemes_explanation)
                .arg(
                    Arg::new("device")
                        .required(true)
                        .num_args(1)
                        .index(1)
                        .help("Storage device ID"),
                )
                .arg(
                    Arg::new("scheme")
                        .long("scheme")
                        .short('s')
                        .num_args(1)
                        .value_parser(scheme_keys)
                        .default_value("random2x")
                        .help("Data sanitization scheme"),
                )
                .arg(
                    Arg::new("verify")
                        .long("verify")
                        .short('v')
                        .num_args(1)
                        .value_parser(["no", "last", "all"])
                        .default_value("last")
                        .help("Verify after completion"),
                )
                .arg(
                    Arg::new("coverage")
                        .long("coverage")
                        .short('c')
                        .help("Verification coverage (in percents)")
                        .num_args(1)
                        .value_parser(value_parser!(f32))
                        .default_value("100.0"),
                )
                .arg(
                    Arg::new("blocksize")
                        .long("blocksize")
                        .short('b')
                        .num_args(1)
                        .default_value("1m")
                        .help("Block size"),
                )
                .arg(
                    Arg::new("offset")
                        .long("offset")
                        .short('o')
                        .num_args(1)
                        .default_value("0")
                        .help("Starting offset (in bytes)"),
                )
                .arg(
                    Arg::new("retries")
                        .long("retries")
                        .short('r')
                        .num_args(1)
                        .value_parser(value_parser!(u32))
                        .default_value("8")
                        .help("Maximum number of retries"),
                )
                .arg(
                    Arg::new("queue-depth")
                        .long("queue-depth")
                        .short('q')
                        .num_args(1)
                        .value_parser(value_parser!(QueueDepth))
                        .default_value("auto")
                        .help("I/O queue depth: 'auto' or 1..256"),
                )
                .arg(
                    Arg::new("hardware-erase")
                        .long("hardware-erase")
                        .num_args(1)
                        .value_parser(value_parser!(HardwareEraseMethod))
                        .default_value("off")
                        .help(
                            "Hardware erase method: off, auto, ata-secure, ata-sanitize, nvme-sanitize, nvme-format",
                        ),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .short('y')
                        .help("Automatically confirm")
                        .action(ArgAction::SetTrue),
                ),
        )
}

pub fn parse_bytes(s: &str) -> Result<u64> {
    let bytes_regex = Regex::new(r"^(?i)(\d+) *(([kmgt])b?)?$")?;
    let captures = bytes_regex.captures(s);

    match captures {
        Some(groups) => {
            let units = groups[1].parse::<u64>().context("Not a number.")?;
            let unit_size = match groups.get(3).map(|m| m.as_str().to_uppercase()) {
                Some(ref u) if u == "K" => 1024,
                Some(ref u) if u == "M" => 1024 * 1024,
                Some(ref u) if u == "G" => 1024 * 1024 * 1024,
                Some(ref u) if u == "T" => 1024 * 1024 * 1024 * 1024,
                _ => 1,
            };

            Ok(units * unit_size)
        }
        _ => Err(anyhow!(
            "Use a number of bytes with optional scale (e.g. 4096, 128k or 2M)."
        )),
    }
}

pub fn parse_block_size(s: &str) -> Result<usize> {
    let bytes = parse_bytes(s)?;
    if !bytes.is_power_of_two() {
        return Err(anyhow!("Should be a power of two."));
    }
    usize::try_from(bytes).context("Block size does not fit into addressable memory.")
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_matches::*;

    #[test]
    fn test_bytes_parser_good() {
        assert_eq!(parse_bytes("4000").unwrap(), 4000);
        assert_eq!(parse_bytes("13k").unwrap(), 13 * 1024);
        assert_eq!(parse_bytes("5M").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_bytes("7g").unwrap(), 7 * 1024 * 1024 * 1024);
        assert_eq!(parse_bytes("11T").unwrap(), 11 * 1024 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_bytes_parser_bad() {
        assert_matches!(parse_bytes(""), Err(_));
        assert_matches!(parse_bytes("xxx"), Err(_));
        assert_matches!(parse_bytes("-128k"), Err(_));
        assert_matches!(parse_bytes("4096.000"), Err(_));
    }

    #[test]
    fn test_block_size_parser_good() {
        let k128 = 128 * 1024;
        let m2 = 2 * 1024 * 1024;

        assert_eq!(parse_block_size("4096").unwrap(), 4096);
        assert_eq!(parse_block_size("128k").unwrap(), k128);
        assert_eq!(parse_block_size("128K").unwrap(), k128);
        assert_eq!(parse_block_size("2m").unwrap(), m2);
        assert_eq!(parse_block_size("2M").unwrap(), m2);
    }

    #[test]
    fn test_block_size_parser_bad() {
        assert_matches!(parse_block_size("4095"), Err(_));
        assert_matches!(parse_block_size("13M"), Err(_));
    }

    #[test]
    fn test_hardware_erase_parser() {
        use std::str::FromStr;

        assert_eq!(
            HardwareEraseMethod::from_str("off").unwrap(),
            HardwareEraseMethod::Off
        );
        assert_eq!(
            HardwareEraseMethod::from_str("AUTO").unwrap(),
            HardwareEraseMethod::Auto
        );
        assert_eq!(
            HardwareEraseMethod::from_str("nvme-sanitize").unwrap(),
            HardwareEraseMethod::NvmeSanitize
        );
        assert!(HardwareEraseMethod::from_str("unknown").is_err());
    }

    #[test]
    fn test_queue_depth_parser() {
        use std::str::FromStr;

        assert_eq!(QueueDepth::from_str("auto").unwrap(), QueueDepth::Auto);
        assert_eq!(QueueDepth::from_str("AUTO").unwrap(), QueueDepth::Auto);
        assert_eq!(QueueDepth::from_str("4").unwrap(), QueueDepth::Fixed(4));
        assert!(QueueDepth::from_str("0").is_err());
        assert!(QueueDepth::from_str("257").is_err());
        assert!(QueueDepth::from_str("deep").is_err());
    }
}
