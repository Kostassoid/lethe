use anyhow::{Context, Result};
use regex::Regex;

pub fn parse_block_size(s: &str) -> Result<usize> {
    let block_size_regex = Regex::new(r"^(?i)(\d+) *(([km])b?)?$").unwrap();
    let captures = block_size_regex.captures(s);

    match captures {
        Some(groups) => {
            let units = groups[1]
                .parse::<usize>()
                .with_context(|| "Not a number.")?;
            let unit_size = match groups.get(3).map(|m| m.as_str().to_uppercase()) {
                Some(ref u) if u == "K" => 1024,
                Some(ref u) if u == "M" => 1024 * 1024,
                _ => 1,
            };

            let bytes_length = (units * unit_size) as usize;
            if bytes_length & (bytes_length - 1) == 0 {
                Ok((units * unit_size) as usize)
            } else {
                Err(anyhow!("Should be a power of two."))
            }
        }
        _ => Err(anyhow!(
            "Use a number of bytes with optional scale (e.g. 4096, 128k or 2M)."
        )),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_matches::*;

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
        assert_matches!(parse_block_size(""), Err(_));
        assert_matches!(parse_block_size("xxx"), Err(_));
        assert_matches!(parse_block_size("-128k"), Err(_));
        assert_matches!(parse_block_size("4096.000"), Err(_));
        assert_matches!(parse_block_size("4095"), Err(_));
    }
}
