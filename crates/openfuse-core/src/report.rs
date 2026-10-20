use serde::Serialize;

use crate::hex;

/// A byte-level view of one input. No report ID, opcode, or payload framing is
/// inferred: callers must preserve those distinctions when collecting captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ByteInspection {
    pub byte_count: usize,
    pub hex: String,
    pub bytes: Vec<u8>,
}

impl ByteInspection {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            byte_count: bytes.len(),
            hex: hex::encode(&bytes),
            bytes,
        }
    }

    pub fn hexdump(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();
        for (line, bytes) in self.bytes.chunks(16).enumerate() {
            let ascii: String = bytes
                .iter()
                .map(|byte| match byte {
                    0x20..=0x7e => *byte as char,
                    _ => '.',
                })
                .collect();
            writeln!(
                output,
                "{:08x}  {:47}  |{ascii}|",
                line * 16,
                hex::encode(bytes)
            )
            .expect("writing to a String cannot fail");
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_preserves_offsets_and_printable_ascii() {
        let report = ByteInspection::new(b"1234567890abcdef\0A\xff".to_vec());
        let dump = report.hexdump();
        assert!(dump.contains("00000000"));
        assert!(dump.contains("|1234567890abcdef|"));
        assert!(dump.contains("00000010  00 41 ff"));
        assert!(dump.contains("|.A.|"));
        assert_eq!(report.byte_count, 19);
    }

    #[test]
    fn empty_binary_input_is_representable() {
        let report = ByteInspection::new(Vec::new());
        assert_eq!(report.byte_count, 0);
        assert_eq!(report.hexdump(), "");
    }
}
