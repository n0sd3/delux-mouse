//! Hex parsing for `send-raw`.

/// Parse "06 09 01", "060901" or "0x06,0x09,0x01" into bytes.
pub fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let digits: String = input
        .replace("0x", "")
        .replace("0X", "")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if digits.is_empty() {
        return Err("hex vazio".into());
    }
    if digits.len() % 2 != 0 {
        return Err(format!("hex com número ímpar de dígitos: {}", digits.len()));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .map_err(|_| format!("hex inválido: {:?}", &digits[i..i + 2]))
        })
        .collect()
}

pub fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaced_and_compact_are_equal() {
        assert_eq!(parse_hex("06 09 01").unwrap(), vec![0x06, 0x09, 0x01]);
        assert_eq!(parse_hex("060901").unwrap(), vec![0x06, 0x09, 0x01]);
        assert_eq!(parse_hex("0x06,0x09,0x01").unwrap(), vec![0x06, 0x09, 0x01]);
    }

    #[test]
    fn odd_length_is_error() {
        assert!(parse_hex("0609 0").is_err());
    }

    #[test]
    fn non_hex_is_error() {
        assert!(parse_hex("zz").is_err());
    }

    #[test]
    fn empty_is_error() {
        assert!(parse_hex("  ").is_err());
    }

    #[test]
    fn formats_lowercase_spaced() {
        assert_eq!(format_hex(&[0x04, 0x0E, 0xff]), "04 0e ff");
    }
}
