use crate::DeviceInfo;
use openfuse_core::protocol::Report64;

#[derive(Debug)]
#[cfg_attr(not(feature = "hid"), allow(dead_code))]
pub(crate) enum TransportError {
    Io(String),
    Framing(String),
}

pub(crate) trait Connection: Send {
    fn write(&mut self, report: Report64) -> Result<(), String>;
    /// At most 10ms blocking; None means no report, never disconnect.
    fn read(&mut self) -> Result<Option<Report64>, TransportError>;
}
pub(crate) trait Backend: Send {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String>;
    fn open(&mut self, device: &DeviceInfo) -> Result<Box<dyn Connection>, String>;
}

#[cfg(feature = "hid")]
pub(crate) fn unavailable(pid: u16, interface: i32) -> Option<String> {
    let reason = if !matches!(pid, 4 | 5 | 10 | 14 | 16 | 18 | 20 | 22 | 38) {
        Some("This product/mode does not use the implemented Mustang read session")
    } else if !matches!(interface, -1 | 0) {
        Some("Only the control interface (0, or unspecified by this OS) is supported")
    } else {
        None
    };
    reason.map(str::to_owned)
}

#[derive(Default)]
pub(crate) struct SystemBackend;
#[cfg(not(feature = "hid"))]
impl Backend for SystemBackend {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
        Err("USB support is not compiled in; launch with --features hid (or use --usb-demo)".into())
    }
    fn open(&mut self, _: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
        Err("USB support is not compiled in".into())
    }
}

#[cfg(feature = "hid")]
mod native {
    use super::*;
    use openfuse_core::{
        catalog::{FENDER_VENDOR_ID, find_device},
        hex,
    };
    fn info(d: &hidapi::DeviceInfo) -> DeviceInfo {
        DeviceInfo {
            id: hex::encode(d.path().to_bytes()),
            name: find_device(d.vendor_id(), d.product_id())
                .map(|d| d.name.to_owned())
                .unwrap_or_else(|| format!("Unknown Fender PID {:04x}", d.product_id())),
            product_id: d.product_id(),
            serial: d
                .serial_number()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            interface_number: d.interface_number(),
            unavailable_reason: unavailable(d.product_id(), d.interface_number()),
        }
    }
    impl Backend for SystemBackend {
        fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
            let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
            Ok(api
                .device_list()
                .filter(|d| d.vendor_id() == FENDER_VENDOR_ID)
                .map(info)
                .collect())
        }
        fn open(&mut self, device: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
            let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
            // Resolve only an exact, freshly enumerated identity. Never open VID/PID first-match.
            let found = api
                .device_list()
                .find(|d| d.vendor_id() == FENDER_VENDOR_ID && info(d) == *device)
                .ok_or("Selected USB interface changed; refresh the device list")?;
            if let Some(reason) = &device.unavailable_reason {
                return Err(reason.clone());
            }
            let handle = api.open_path(found.path()).map_err(|e| e.to_string())?;
            // Clear bounded pre-session input before detection. This reduces stale
            // replies; the protocol still cannot prove same-command ACK freshness.
            drain_input(|| {
                let mut bytes = [0; 65];
                handle
                    .read_timeout(&mut bytes, 0)
                    .map_err(|e| e.to_string())
            })?;
            Ok(Box::new(HidConnection(handle)))
        }
    }
    struct HidConnection(hidapi::HidDevice);
    impl Connection for HidConnection {
        fn write(&mut self, report: Report64) -> Result<(), String> {
            let bytes = report.to_hid_output();
            let count = self.0.write(&bytes).map_err(|e| e.to_string())?;
            validate_write(count)
        }
        fn read(&mut self) -> Result<Option<Report64>, TransportError> {
            // One extra byte detects unexpected numbered/oversized reports.
            let mut bytes = [0; 65];
            let count = self
                .0
                .read_timeout(&mut bytes, 10)
                .map_err(|e| TransportError::Io(e.to_string()))?;
            decode_read(&bytes[..count])
        }
    }
}

#[cfg(any(feature = "hid", test))]
fn validate_write(count: usize) -> Result<(), String> {
    if count == 65 {
        Ok(())
    } else {
        Err(format!("Short HID write: {count}/65"))
    }
}
#[cfg(any(feature = "hid", test))]
fn decode_read(bytes: &[u8]) -> Result<Option<Report64>, TransportError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    Report64::try_from(bytes)
        .map(Some)
        .map_err(|e| TransportError::Framing(format!("Invalid HID input: {e}")))
}
#[cfg(any(feature = "hid", test))]
fn drain_input(mut read: impl FnMut() -> Result<usize, String>) -> Result<(), String> {
    for _ in 0..256 {
        if read()? == 0 {
            return Ok(());
        }
    }
    Err("USB input did not become idle within 256 reports; no detect request sent".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_boundary_rejects_truncation_padding_and_short_writes() {
        assert!(decode_read(&[]).unwrap().is_none());
        let mut ack = [0; 64];
        ack[1] = 0xc1;
        ack[3] = 4;
        assert_eq!(decode_read(&ack).unwrap().unwrap().as_bytes(), &ack);
        for length in [1, 63, 65, 128] {
            assert!(decode_read(&vec![0; length]).is_err());
        }
        assert!(validate_write(65).is_ok());
        for count in [0, 1, 64, 66] {
            assert!(validate_write(count).is_err());
        }
    }
    #[test]
    fn stale_input_drain_is_bounded_and_propagates_errors() {
        let mut lengths = [64, 64, 0].into_iter();
        assert!(drain_input(|| Ok(lengths.next().unwrap())).is_ok());
        let mut count = 0;
        assert!(
            drain_input(|| {
                count += 1;
                Ok(64)
            })
            .is_err()
        );
        assert_eq!(count, 256);
        assert_eq!(
            drain_input(|| Err("disconnected".into())).unwrap_err(),
            "disconnected"
        );
    }
}
