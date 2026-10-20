//! Strict byte parsing for pasted captures, without protocol assumptions.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HexError {
    Empty,
    InvalidDigit { offset: usize, character: char },
    OddDigitCount { offset: usize },
    EmptyPrefix { offset: usize },
}

impl fmt::Display for HexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "no hexadecimal bytes were supplied"),
            Self::InvalidDigit { offset, character } => {
                write!(
                    formatter,
                    "invalid hex character {character:?} at byte offset {offset}"
                )
            }
            Self::OddDigitCount { offset } => {
                write!(
                    formatter,
                    "incomplete hexadecimal byte at byte offset {offset}"
                )
            }
            Self::EmptyPrefix { offset } => {
                write!(
                    formatter,
                    "hexadecimal prefix without digits at byte offset {offset}"
                )
            }
        }
    }
}

impl std::error::Error for HexError {}

/// Accepts contiguous hex or byte groups separated by ASCII whitespace, `:` or
/// `-`. Each group may begin with `0x`. Every group must contain complete bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, HexError> {
    let mut output = Vec::new();
    let mut start = 0;
    for (offset, character) in input.char_indices() {
        if character.is_ascii_whitespace() || matches!(character, ':' | '-') {
            decode_group(&input[start..offset], start, &mut output)?;
            start = offset + character.len_utf8();
        }
    }
    decode_group(&input[start..], start, &mut output)?;
    if output.is_empty() {
        Err(HexError::Empty)
    } else {
        Ok(output)
    }
}

fn decode_group(group: &str, offset: usize, output: &mut Vec<u8>) -> Result<(), HexError> {
    if group.is_empty() {
        return Ok(());
    }
    let (digits, offset) = if group.starts_with("0x") || group.starts_with("0X") {
        if group.len() == 2 {
            return Err(HexError::EmptyPrefix { offset });
        }
        (&group[2..], offset + 2)
    } else {
        (group, offset)
    };
    let mut high = None;
    for (index, character) in digits.char_indices() {
        let value = character
            .to_digit(16)
            .filter(|_| character.is_ascii())
            .ok_or(HexError::InvalidDigit {
                offset: offset + index,
                character,
            })? as u8;
        if let Some(high) = high.take() {
            output.push((high << 4) | value);
        } else {
            high = Some(value);
        }
    }
    if high.is_some() {
        return Err(HexError::OddDigitCount {
            offset: offset + digits.len() - 1,
        });
    }
    Ok(())
}

pub fn encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len().saturating_mul(3));
    for (index, byte) in bytes.iter().enumerate() {
        if index != 0 {
            output.push(' ');
        }
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_capture_formats() {
        for input in ["00aBff", "00 AB ff", "0x00:0XAB-ff", "\n00\tAB\r\nff "] {
            assert_eq!(decode(input).unwrap(), [0, 171, 255]);
        }
    }

    #[test]
    fn all_byte_values_round_trip() {
        let bytes: Vec<_> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn refuses_corrupt_or_ambiguous_input() {
        assert_eq!(decode(" : \t-"), Err(HexError::Empty));
        assert_eq!(decode("00 0x"), Err(HexError::EmptyPrefix { offset: 3 }));
        assert_eq!(decode("00 a b"), Err(HexError::OddDigitCount { offset: 3 }));
        assert_eq!(decode("0x1"), Err(HexError::OddDigitCount { offset: 2 }));
        assert_eq!(
            decode("00 ø"),
            Err(HexError::InvalidDigit {
                offset: 3,
                character: 'ø'
            })
        );
        assert!(decode("[01,02]").is_err());
        assert!(decode("0x010x02").is_err());
    }
}
