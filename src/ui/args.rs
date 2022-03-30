use anyhow::{Context, Result};
use regex::Regex;

pub fn parse_bytes(s: &str) -> Result<u64> {
    let bytes_regex = Regex::new(r"^(?i)(\d+) *(([kmgt])b?)?$").unwrap();
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
    parse_bytes(s).and_then(|bytes| {
        if bytes & (bytes - 1) == 0 {
            Ok(bytes as usize)
        } else {
            Err(anyhow!("Should be a power of two."))
        }
    })
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
}
