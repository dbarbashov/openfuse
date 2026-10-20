use super::{Event, Trace};
use openfuse_core::usb_session::{SessionMachine, SessionStatus};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReplayResult {
    pub trace_sha256: String,
    pub product_id: u16,
    pub firmware: String,
    pub tx_verified: usize,
    pub rx_consumed: usize,
    pub events_consumed: usize,
    pub events_not_replayed: usize,
    pub preset_reports: usize,
    pub hardware_validated: bool,
}
/// Runs the production read-session automaton, never a USB backend. Only the
/// first successful detect/version/current exchange is claimed as replayed.
/// Later editing, saves, reconnects and audio require separate acceptance.
pub fn replay_bootstrap(trace: &Trace) -> Result<ReplayResult, String> {
    trace.validate()?;
    if trace.dropped_events != 0 {
        return Err("Truncated trace cannot pass replay".into());
    }
    let Some(first) = trace.entries.first() else {
        return Err("Trace is empty".into());
    };
    let Event::Open { profile } = &first.event else {
        return Err("Replay requires an initial successful open".into());
    };
    let mut engine = SessionMachine::for_pid(profile.product_id).map_err(|e| e.to_string())?;
    engine.start(first.at_ms).map_err(|e| e.to_string())?;
    let mut tx = 0;
    let mut rx = 0;
    for (index, entry) in trace.entries.iter().enumerate().skip(1) {
        engine.tick(entry.at_ms);
        if let SessionStatus::Failed(error) = engine.status() {
            return Err(format!("Replay event {index}: {error}"));
        }
        match &entry.event {
            Event::Tx {
                bytes,
                succeeded: true,
            } => {
                let expected = engine
                    .take_outgoing()
                    .ok_or_else(|| format!("Unexpected transmit at event {index}"))?;
                if expected.as_bytes().as_slice() != bytes {
                    return Err(format!("Transmit mismatch at event {index}"));
                }
                tx += 1;
            }
            Event::Rx { bytes } => {
                engine
                    .on_bytes(bytes, entry.at_ms)
                    .map_err(|e| e.to_string())?;
                rx += 1;
            }
            _ => {
                return Err(format!(
                    "Connection ended or failed before bootstrap completed at event {index}"
                ));
            }
        }
        if let SessionStatus::Failed(error) = engine.status() {
            return Err(format!("Replay event {index}: {error}"));
        }
        if engine.status() == &SessionStatus::Ready {
            let version = engine
                .version()
                .ok_or("Replay did not produce a firmware version")?;
            let snapshot = engine.snapshot().ok_or("Replay did not produce a preset")?;
            return Ok(ReplayResult {
                trace_sha256: trace.digest()?,
                product_id: profile.product_id,
                firmware: format!(
                    "{}.{} build {}",
                    version.major, version.minor, version.build
                ),
                tx_verified: tx,
                rx_consumed: rx,
                events_consumed: index + 1,
                events_not_replayed: trace.entries.len() - index - 1,
                preset_reports: snapshot.reports.len(),
                hardware_validated: false,
            });
        }
    }
    engine.tick(trace.ended_ms);
    Err(format!("Incomplete bootstrap: {:?}", engine.status()))
}
