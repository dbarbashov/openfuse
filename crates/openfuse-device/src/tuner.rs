//! Tuner owns a session while enabled. Control state echoes, not ACKs, confirm it.
use openfuse_core::{
    controls::{self, TunerReading},
    protocol::Report64,
};
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunerStatus {
    pub enabled: bool,
    pub muted: bool,
    pub pending: bool,
    pub confirmed: bool,
    pub stale: bool,
    pub reading: Option<TunerReading>,
    pub detail: String,
    pub error: Option<String>,
}
impl Default for TunerStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            muted: false,
            pending: false,
            confirmed: false,
            stale: true,
            reading: None,
            detail: "Tuner is off; amplifier state has not been observed".into(),
            error: None,
        }
    }
}
impl TunerStatus {
    pub fn exclusive(&self) -> bool {
        self.enabled || self.pending || self.error.is_some()
    }
}
#[derive(Default)]
pub(crate) struct Tuner {
    status: TunerStatus,
    outgoing: Option<Report64>,
    deadline: Option<u64>,
    last_pitch: Option<u64>,
}
impl Tuner {
    pub(crate) fn status(&self) -> &TunerStatus {
        &self.status
    }
    pub(crate) fn request(&mut self, pid: u16, enabled: bool, muted: bool) -> Result<(), String> {
        let report = controls::tuner_control(pid, enabled, muted).map_err(|e| e.to_string())?;
        self.outgoing = Some(report);
        self.deadline = None;
        self.last_pitch = None;
        self.status = TunerStatus {
            enabled,
            muted: enabled && muted,
            pending: true,
            confirmed: false,
            stale: true,
            reading: None,
            detail: if enabled {
                "Starting tuner; waiting for device state"
            } else {
                "Stopping tuner; waiting for device state"
            }
            .into(),
            error: None,
        };
        Ok(())
    }
    pub(crate) fn take_outgoing(&mut self, now: u64) -> Option<Report64> {
        let report = self.outgoing.take()?;
        self.deadline = Some(now.saturating_add(5000));
        Some(report)
    }
    pub(crate) fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.outgoing = None;
        self.deadline = None;
        self.last_pitch = None;
        self.status.pending = false;
        self.status.confirmed = false;
        self.status.stale = true;
        self.status.reading = None;
        self.status.detail = message.clone();
        self.status.error = Some(message);
    }
    pub(crate) fn lost(&mut self) {
        if self.status.exclusive() {
            self.fail(
                "Connection lost; tuner/mute state is unknown. Stop explicitly after reconnecting",
            );
        }
    }
    pub(crate) fn tick(&mut self, now: u64) {
        if self.deadline.is_some_and(|d| now >= d) {
            self.fail("No matching tuner state within 5 seconds; delivery is uncertain. Stop explicitly to recover");
        }
        if self.status.enabled
            && self
                .last_pitch
                .is_some_and(|t| now.saturating_sub(t) >= 1500)
        {
            self.status.stale = true;
        }
    }
    pub(crate) fn on_report(&mut self, report: Report64, now: u64) {
        self.tick(now);
        let b = report.as_bytes();
        if b[0] == 0x0a {
            if self.outgoing.is_some() {
                return;
            }
            let enabled = b[2] != 0;
            let muted = b[4] != 0;
            if self.status.pending && (enabled != self.status.enabled || muted != self.status.muted)
            {
                return;
            }
            self.status.enabled = enabled;
            self.status.muted = muted;
            self.status.pending = false;
            self.status.confirmed = true;
            self.status.error = None;
            self.deadline = None;
            self.status.detail = if enabled {
                "Tuner state confirmed; waiting for fresh pitch"
            } else {
                "Tuner off confirmed"
            }
            .into();
            if !enabled {
                self.status.reading = None;
                self.status.stale = true;
                self.last_pitch = None;
            }
        } else if b[0] == 0x0b
            && self.status.enabled
            && !self.status.pending
            && self.status.confirmed
            && let Some(reading) = TunerReading::decode(&report)
        {
            self.status.reading = Some(reading);
            self.last_pitch = Some(now);
            self.status.stale = false;
            self.status.detail = "Pitch received from amplifier".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(id: u8, a: u8, b: u8, c: u8) -> Report64 {
        let mut raw = [0; 64];
        raw[0] = id;
        raw[1] = 1;
        raw[2] = a;
        raw[3] = b;
        raw[4] = c;
        Report64::from_bytes(raw)
    }
    #[test]
    fn only_matching_control_state_confirms_and_raw_unknown_pitch_is_retained() {
        let mut t = Tuner::default();
        t.request(20, true, true).unwrap();
        t.on_report(report(10, 1, 1, 1), 0);
        assert!(t.status().pending);
        t.take_outgoing(1).unwrap();
        t.on_report(report(0, 10, 0, 0), 2);
        assert!(t.status().pending);
        t.on_report(report(11, 0, 0, 0), 3);
        assert!(t.status().reading.is_none());
        t.on_report(report(10, 1, 1, 0), 4);
        assert!(t.status().pending);
        t.on_report(report(10, 1, 1, 1), 5);
        assert!(t.status().confirmed);
        t.on_report(report(11, 255, 128, 0), 6);
        let r = t.status().reading.unwrap();
        assert_eq!(r.offset, -128);
        assert_eq!(r.note, controls::TunerNote::Unknown(255));
        t.on_report(report(11, 12, 0, 0), 7);
        assert_eq!(
            t.status().reading.unwrap().note,
            controls::TunerNote::Invalid
        );
        t.tick(1507);
        assert!(t.status().stale);
        t.request(20, false, false).unwrap();
        assert!(t.status().reading.is_none());
        t.take_outgoing(1510);
        t.on_report(report(10, 1, 1, 1), 1511);
        assert!(t.status().pending);
        t.on_report(report(10, 0, 0, 0), 1512);
        assert!(!t.status().exclusive());
    }
}
