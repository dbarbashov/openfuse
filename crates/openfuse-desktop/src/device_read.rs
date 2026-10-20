//! Immutable read preview. It owns the chosen bytes across the dirty-document guard.
use openfuse_core::{preset::PresetDocument, protocol::Report64, wire_preset};
use openfuse_device::{LiveBinding, WireCapture};

#[derive(Clone)]
pub struct ReadDraft {
    pub capture: WireCapture,
    pub document: PresetDocument,
    pub warnings: Vec<String>,
    pub offline_file: bool,
}
impl ReadDraft {
    pub fn decode(capture: WireCapture) -> Result<Self, String> {
        if capture.reports.len() > 512 {
            return Err("Capture exceeds 512 preset reports".into());
        }
        let reports = capture
            .reports
            .iter()
            .map(|bytes| Report64::try_from(bytes.as_slice()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let decoded =
            wire_preset::decode(capture.device.product_id, &reports).map_err(|e| e.to_string())?;
        Ok(Self {
            capture,
            document: decoded.document,
            warnings: decoded.warnings,
            offline_file: false,
        })
    }
    pub fn source_label(&self) -> String {
        format!(
            "{} · read {} · PID {:04x}",
            self.capture.device.name, self.capture.generation, self.capture.device.product_id
        )
    }
}

pub struct DeviceOrigin {
    pub read: ReadDraft,
    pub binding: LiveBinding,
    pub invalidated: Option<String>,
}
impl From<ReadDraft> for DeviceOrigin {
    fn from(read: ReadDraft) -> Self {
        Self { binding: LiveBinding {device_id: read.capture.device.id.clone(), generation: read.capture.generation}, invalidated: read.offline_file.then(|| "Saved diagnostic captures open offline. Read the connected amplifier to enable live parameters.".into()), read }
    }
}
