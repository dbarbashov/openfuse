//! Device identities recovered from the shipped software.
//!
//! An identity in this catalog is evidence of software recognition, not proof
//! that openfuse implements or has tested that device's protocol.

use serde::Serialize;

/// Fender's USB vendor identity observed in the original FUSE distribution.
pub const FENDER_VENDOR_ID: u16 = 0x1ed8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DeviceIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: &'static str,
    pub bootloader: bool,
    /// Compatibility evidence status; recognition does not establish support.
    pub evidence: &'static str,
}

const fn identity(product_id: u16, name: &'static str, bootloader: bool) -> DeviceIdentity {
    DeviceIdentity {
        vendor_id: FENDER_VENDOR_ID,
        product_id,
        name,
        bootloader,
        evidence: "Not hardware-validated",
    }
}

/// Product IDs from DevComms DevicePid attributes/constructor switches and
/// matching installer kernel-extension plists. IDs are written in decimal here.
pub const DEVICES: &[DeviceIdentity] = &[
    identity(2, "G-DEC", false),
    identity(3, "G-DEC mass storage", false),
    identity(4, "Mustang I/II", false),
    identity(5, "Mustang III/IV/V", false),
    identity(6, "Mustang I/II bootloader", true),
    identity(7, "Mustang III/IV/V bootloader", true),
    identity(10, "Bronco", false),
    identity(11, "Bronco bootloader", true),
    identity(12, "Acoustasonic", false),
    identity(13, "Acoustasonic bootloader", true),
    identity(14, "Super Champ X2", false),
    identity(15, "Super Champ X2 bootloader", true),
    identity(16, "Mustang Mini", false),
    identity(17, "Mustang Mini bootloader", true),
    identity(18, "Mustang Floor", false),
    identity(19, "Mustang Floor bootloader", true),
    identity(20, "Mustang I/II V2", false),
    identity(21, "Mustang I/II V2 bootloader", true),
    identity(22, "Mustang III/IV/V V2", false),
    identity(23, "Mustang III/IV/V V2 bootloader", true),
    identity(38, "Passport Mini", false),
    identity(39, "Passport Mini bootloader", true),
];

pub fn find_device(vendor_id: u16, product_id: u16) -> Option<&'static DeviceIdentity> {
    DEVICES
        .iter()
        .find(|device| device.vendor_id == vendor_id && device.product_id == product_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_unique_identifiers_and_does_not_match_other_vendors() {
        let mut ids = std::collections::HashSet::new();
        for device in DEVICES {
            assert!(ids.insert((device.vendor_id, device.product_id)));
            assert_eq!(
                find_device(device.vendor_id, device.product_id),
                Some(device)
            );
            assert!(find_device(0, device.product_id).is_none());
            assert!(!device.evidence.is_empty());
        }
        assert!(find_device(FENDER_VENDOR_ID, 0xffff).is_none());
    }
}
