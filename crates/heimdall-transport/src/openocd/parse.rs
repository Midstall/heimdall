//! Parsing helpers for OpenOCD command responses.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected response shape: {0}")]
    Shape(String),
    #[error("could not parse hex value `{0}`")]
    BadHex(String),
    #[error("could not parse byte count from `{0}`")]
    BadByteCount(String),
}

/// Parse a register-read response like:
///   `xN (/64): 0x000000000000002a`
///   `pc (/32): 0x80000010`
///   `dpc (/64): 0x000000008000001c`
/// Returns the value as u64.
pub fn parse_reg_response(raw: &str) -> Result<u64, ParseError> {
    let trimmed = raw.trim();
    // Find "0x" and parse hex from that point.
    let hex_start = trimmed
        .rfind("0x")
        .ok_or_else(|| ParseError::Shape(trimmed.to_string()))?;
    let hex_part = &trimmed[hex_start + 2..];
    // Take only valid hex characters until the first non-hex char.
    let end = hex_part
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(hex_part.len());
    let hex = &hex_part[..end];
    if hex.is_empty() {
        return Err(ParseError::BadHex(trimmed.to_string()));
    }
    u64::from_str_radix(hex, 16).map_err(|_| ParseError::BadHex(hex.to_string()))
}

/// Parse a `load_image` response. OpenOCD typically prints lines like:
///   `123 bytes written at address 0x80000000`
///   `downloaded 123 bytes in 0.123456s (1.234 KiB/s)`
/// Returns the number of bytes loaded.
pub fn parse_load_image_response(raw: &str) -> Result<usize, ParseError> {
    // Look for "N bytes" anywhere in the output.
    let lower = raw.to_ascii_lowercase();
    let idx = lower
        .find(" bytes")
        .ok_or_else(|| ParseError::Shape(raw.to_string()))?;
    let prefix = &lower[..idx];
    // Walk backwards over whitespace then digits.
    let digits: String = prefix
        .chars()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits.is_empty() {
        return Err(ParseError::BadByteCount(raw.to_string()));
    }
    digits.parse().map_err(|_| ParseError::BadByteCount(digits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_rv64_hex() {
        let raw = "x10 (/64): 0x000000000000002a";
        assert_eq!(parse_reg_response(raw).unwrap(), 0x2a);
    }

    #[test]
    fn reg_pc() {
        let raw = "pc (/32): 0x80000010";
        assert_eq!(parse_reg_response(raw).unwrap(), 0x8000_0010);
    }

    #[test]
    fn reg_dpc_rv64() {
        let raw = "dpc (/64): 0x000000008000001c";
        assert_eq!(parse_reg_response(raw).unwrap(), 0x8000_001c);
    }

    #[test]
    fn reg_trailing_text() {
        // Some openocd versions append extra info; trailing non-hex stops parsing cleanly.
        let raw = "x1 (/64): 0x00000000deadbeef (modified)";
        assert_eq!(parse_reg_response(raw).unwrap(), 0xdead_beef);
    }

    #[test]
    fn reg_no_hex_marker_errors() {
        let raw = "no hex here";
        assert!(matches!(parse_reg_response(raw), Err(ParseError::Shape(_))));
    }

    #[test]
    fn reg_bad_hex_errors() {
        let raw = "x1 (/64): 0x";
        assert!(matches!(
            parse_reg_response(raw),
            Err(ParseError::BadHex(_))
        ));
    }

    #[test]
    fn load_image_downloaded_form() {
        let raw = "downloaded 1024 bytes in 0.005s (200 KiB/s)";
        assert_eq!(parse_load_image_response(raw).unwrap(), 1024);
    }

    #[test]
    fn load_image_written_form() {
        let raw = "123 bytes written at address 0x80000000";
        assert_eq!(parse_load_image_response(raw).unwrap(), 123);
    }

    #[test]
    fn load_image_missing_bytes_keyword_errors() {
        let raw = "loaded file at 0x80000000";
        assert!(matches!(
            parse_load_image_response(raw),
            Err(ParseError::Shape(_))
        ));
    }
}
