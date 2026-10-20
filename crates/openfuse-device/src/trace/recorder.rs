use super::{Entry, Event, MAX_EVENTS, Origin, Profile, Trace};
use crate::{
    DeviceInfo,
    backend::{Backend, Connection, TransportError},
};
use openfuse_core::protocol::Report64;
use std::{
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Recorder(Arc<Mutex<State>>);
struct State {
    origin: Origin,
    active_connections: usize,
    recording: Option<(Instant, Trace)>,
}
impl Recorder {
    pub(crate) fn new(origin: Origin) -> Self {
        Self(Arc::new(Mutex::new(State {
            origin,
            active_connections: 0,
            recording: None,
        })))
    }
    /// Does not enumerate, open or send. Arm before explicitly connecting.
    pub fn start(&self) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if state.active_connections != 0 || state.recording.is_some() {
            return Err(
                "Disconnect before starting a new trace; stop any active recording first".into(),
            );
        }
        let trace = Trace {
            schema: 1,
            origin: state.origin,
            app_version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            ended_ms: 0,
            dropped_events: 0,
            entries: Vec::new(),
        };
        state.recording = Some((Instant::now(), trace));
        Ok(())
    }
    pub fn progress(&self) -> Option<(usize, u64)> {
        self.0
            .lock()
            .unwrap()
            .recording
            .as_ref()
            .map(|(_, t)| (t.entries.len(), t.dropped_events))
    }
    pub fn stop(&self) -> Option<Trace> {
        self.0
            .lock()
            .unwrap()
            .recording
            .take()
            .map(|(start, mut trace)| {
                trace.ended_ms = millis(start);
                trace
            })
    }
}
fn millis(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}
impl State {
    fn record(&mut self, event: Event) {
        if let Some((start, trace)) = &mut self.recording {
            if trace.entries.len() == MAX_EVENTS {
                trace.dropped_events = trace.dropped_events.saturating_add(1);
            } else {
                trace.entries.push(Entry {
                    at_ms: millis(*start),
                    event,
                });
            }
        }
    }
}
pub(crate) struct RecordingBackend {
    pub inner: Box<dyn Backend>,
    pub recorder: Recorder,
}
impl Backend for RecordingBackend {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
        self.inner.enumerate()
    }
    fn open(&mut self, device: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
        // Reserve while opening: another thread cannot arm a recording midway
        // through native open/drain and accidentally claim a complete session.
        self.recorder.0.lock().unwrap().active_connections += 1;
        match self.inner.open(device) {
            Ok(inner) => {
                self.recorder.0.lock().unwrap().record(Event::Open {
                    profile: Profile {
                        product_id: device.product_id,
                        interface_number: device.interface_number,
                    },
                });
                Ok(Box::new(RecordingConnection {
                    inner,
                    recorder: self.recorder.clone(),
                }))
            }
            Err(error) => {
                let mut state = self.recorder.0.lock().unwrap();
                state.active_connections -= 1;
                state.record(Event::OpenFailed {
                    profile: Profile {
                        product_id: device.product_id,
                        interface_number: device.interface_number,
                    },
                });
                Err(error)
            }
        }
    }
}
struct RecordingConnection {
    inner: Box<dyn Connection>,
    recorder: Recorder,
}
impl Connection for RecordingConnection {
    fn write(&mut self, report: Report64) -> Result<(), String> {
        let result = self.inner.write(report);
        self.recorder.0.lock().unwrap().record(Event::Tx {
            bytes: report.as_bytes().to_vec(),
            succeeded: result.is_ok(),
        });
        result
    }
    fn read(&mut self) -> Result<Option<Report64>, TransportError> {
        let result = self.inner.read();
        let event = match &result {
            Ok(Some(report)) => Some(Event::Rx {
                bytes: report.as_bytes().to_vec(),
            }),
            Err(error) => Some(Event::Fault {
                framing: matches!(error, TransportError::Framing(_)),
            }),
            Ok(None) => None,
        };
        if let Some(event) = event {
            self.recorder.0.lock().unwrap().record(event);
        }
        result
    }
}
impl Drop for RecordingConnection {
    fn drop(&mut self) {
        let mut state = self.recorder.0.lock().unwrap();
        state.record(Event::Close);
        state.active_connections -= 1;
    }
}
