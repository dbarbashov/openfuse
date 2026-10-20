//! FUSE DevComms framing reconstructed from the shipped managed assembly.
//!
//! Packet framing does not establish validated hardware support.
//! Original DevComms SHA-256:
//! `5e4980f956c83cfe6903f0c17f19767e64798245fcd85569713f9c07a2ca081b`.
//! These codecs do not perform USB I/O or establish a device session.

use std::fmt;

pub const REPORT_SIZE: usize = 64;
pub const HID_OUTPUT_SIZE: usize = 65;
pub const FLAG_START: u8 = 0x40;
pub const FLAG_END: u8 = 0x80;
pub const FLAG_ACK: u8 = 0x02;
pub const FLAG_SECONDARY: u8 = 0x04;
pub const KNOWN_FLAGS: u8 = FLAG_START | FLAG_END | FLAG_ACK | FLAG_SECONDARY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report64([u8; REPORT_SIZE]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingError {
    Length { expected: usize, actual: usize },
    HidReportId(u8),
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => {
                write!(formatter, "expected {expected} bytes, received {actual}")
            }
            Self::HidReportId(id) => {
                write!(formatter, "expected HID output report ID 0, received {id}")
            }
        }
    }
}

impl std::error::Error for FramingError {}

impl TryFrom<&[u8]> for Report64 {
    type Error = FramingError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        bytes
            .try_into()
            .map(Self)
            .map_err(|_| FramingError::Length {
                expected: REPORT_SIZE,
                actual: bytes.len(),
            })
    }
}

impl Report64 {
    pub fn from_bytes(bytes: [u8; REPORT_SIZE]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; REPORT_SIZE] {
        &self.0
    }

    pub fn message_id(&self) -> u8 {
        self.0[0]
    }

    /// Returns all bits, including those whose meaning has not been established.
    pub fn flags(&self) -> u8 {
        self.0[1]
    }

    pub fn unknown_flags(&self) -> u8 {
        self.flags() & !KNOWN_FLAGS
    }

    pub fn has_flag(&self, flag: u8) -> bool {
        self.flags() & flag == flag
    }

    /// Bytes after the common header, without interpreting message-specific fields.
    pub fn body(&self) -> &[u8] {
        &self.0[2..]
    }

    /// Exact host output convention in HidApiInteropCommLayer.SendData:
    /// prepend a zero report ID to the unmodified 64-byte raw message.
    pub fn to_hid_output(&self) -> [u8; HID_OUTPUT_SIZE] {
        let mut output = [0; HID_OUTPUT_SIZE];
        output[1..].copy_from_slice(&self.0);
        output
    }

    pub fn from_hid_output(bytes: &[u8]) -> Result<Self, FramingError> {
        if bytes.len() != HID_OUTPUT_SIZE {
            return Err(FramingError::Length {
                expected: HID_OUTPUT_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != 0 {
            return Err(FramingError::HidReportId(bytes[0]));
        }
        Self::try_from(&bytes[1..])
    }

    /// Constructor bytes observed in DevComms; generating this does not send it.
    pub fn device_detect() -> Self {
        let mut bytes = [0; REPORT_SIZE];
        bytes[1] = 0xc3;
        Self(bytes)
    }

    /// Constructor bytes observed in DevComms; session prerequisites unverified.
    pub fn version_request() -> Self {
        let mut bytes = [0; REPORT_SIZE];
        bytes[0] = 0x1a;
        bytes[1] = 0xc1;
        Self(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_bytes_match_recovered_constants() {
        for (report, header) in [
            (Report64::device_detect(), [0, 0xc3]),
            (Report64::version_request(), [0x1a, 0xc1]),
        ] {
            assert_eq!(&report.as_bytes()[..2], &header);
            assert!(report.body().iter().all(|byte| *byte == 0));
            assert!(report.has_flag(FLAG_START | FLAG_END));
            assert_eq!(report.unknown_flags(), 1);
        }
        assert!(Report64::device_detect().has_flag(FLAG_ACK));
        assert!(!Report64::version_request().has_flag(FLAG_ACK));
    }

    #[test]
    fn hid_prefix_round_trip_preserves_every_byte_and_unknown_flags() {
        let raw = std::array::from_fn(|index| (index * 3 + 1) as u8);
        let report = Report64::from_bytes(raw);
        let output = report.to_hid_output();
        assert_eq!(output[0], 0);
        assert_eq!(&output[1..], &raw);
        assert_eq!(Report64::from_hid_output(&output).unwrap(), report);
        assert_eq!(Report64::try_from(raw.as_slice()).unwrap().as_bytes(), &raw);
    }

    #[test]
    fn framing_rejects_truncation_padding_and_nonzero_hid_id() {
        for length in [0, 1, 63, 65, 128] {
            assert!(Report64::try_from(vec![0; length].as_slice()).is_err());
        }
        for length in [0, 64, 66] {
            assert!(Report64::from_hid_output(&vec![0; length]).is_err());
        }
        let mut output = [0; HID_OUTPUT_SIZE];
        output[0] = 1;
        assert_eq!(
            Report64::from_hid_output(&output),
            Err(FramingError::HidReportId(1))
        );
    }
}
