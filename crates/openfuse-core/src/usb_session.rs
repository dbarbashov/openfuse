//! Bounded, deterministic Mustang-family read sessions. No USB I/O occurs here.
//!
//! Deadlines begin
//! when a command is queued. The caller must write each `take_outgoing` report
//! exactly once, fail the session on a write error, and discard/drain the old
//! transport before starting a new connection. The protocol has no transaction
//! nonce: a stale same-command ACK cannot be distinguished on an undrained link.
use std::{collections::VecDeque, fmt};

use crate::{messages, protocol::Report64};

pub const SESSION_PIDS: &[u16] = &[
    0x0004, 0x0005, 0x000a, 0x000e, 0x0010, 0x0012, 0x0014, 0x0016, 0x0026,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    UnsupportedPid(u16),
    Busy,
    Timeout,
    NegativeAck(u8),
    UnsupportedFirmware,
    MalformedReport(usize),
    IncompletePreset,
    DuplicateCurrentPreset,
    InterleavedPreset,
    DuplicateModule(u8),
    SyncOverflow,
    Transport(String),
}
impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPid(pid) => {
                write!(f, "no evidenced read-session protocol for PID {pid:04x}")
            }
            Self::Busy => f.write_str("session is not ready for this operation"),
            Self::Timeout => f.write_str("device response timed out"),
            Self::NegativeAck(status) => write!(f, "device rejected detection (status {status})"),
            Self::UnsupportedFirmware => {
                f.write_str("firmware is below the original editor's sync minimum")
            }
            Self::MalformedReport(n) => write!(f, "expected 64 input bytes, received {n}"),
            Self::IncompletePreset => {
                f.write_str("sync ended without one complete current preset and all five modules")
            }
            Self::InterleavedPreset => {
                f.write_str("another preset began before the current preset completed")
            }
            Self::DuplicateCurrentPreset => {
                f.write_str("ambiguous duplicate current preset in one sync")
            }
            Self::DuplicateModule(kind) => {
                write!(f, "duplicate module type {kind} in current preset")
            }
            Self::SyncOverflow => f.write_str("sync exceeded its report bound"),
            Self::Transport(error) => write!(f, "USB transport: {error}"),
        }
    }
}
impl std::error::Error for SessionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Disconnected,
    Detecting,
    Version,
    Syncing,
    Ready,
    Failed(SessionError),
}

/// Conservative application policy, not timing requirements measured on an amp.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    pub detect_timeout_ms: u64,
    pub version_timeout_ms: u64,
    pub sync_timeout_ms: u64,
    pub max_sync_reports: usize,
    pub max_received_reports: usize,
}
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            detect_timeout_ms: 6_000,
            version_timeout_ms: 6_000,
            sync_timeout_ms: 15_000,
            max_sync_reports: 512,
            max_received_reports: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceVersion {
    pub major: u8,
    pub minor: u8,
    pub build: u16,
    pub build_date: [u8; 16],
    pub raw: Report64,
}

/// Model identifiers are decoded; parameter payloads remain exact wire bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireModule {
    pub data_type: u8,
    pub model_id: u16,
    pub raw: Report64,
}

/// An atomic wire snapshot, not a reconstructed .FUSE document. `reports`
/// retains the Name-to-Complete current-preset reports, including unknown types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetSnapshot {
    pub name: String,
    pub position: u16,
    pub modified: bool,
    pub modules: Vec<WireModule>,
    pub reports: Vec<Report64>,
    pub captured_at_ms: u64,
}

#[derive(Debug)]
struct PendingPreset {
    snapshot: PresetSnapshot,
    complete: bool,
}

#[derive(Debug)]
pub struct SessionMachine {
    pid: u16,
    config: SessionConfig,
    status: SessionStatus,
    deadline: Option<u64>,
    outgoing: Option<Report64>,
    version: Option<DeviceVersion>,
    snapshot: Option<PresetSnapshot>,
    pending: Option<PendingPreset>,
    sync_reports: usize,
    received: VecDeque<Report64>,
    dropped_reports: u64,
}
impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}
impl SessionMachine {
    /// Offline/default model profile: Mustang I/II V2. Device workers must use
    /// `for_pid` with the actual selected identity instead.
    pub fn new() -> Self {
        Self::for_pid(0x0014).expect("known profile")
    }
    pub fn for_pid(pid: u16) -> Result<Self, SessionError> {
        Self::with_config(pid, SessionConfig::default())
    }
    pub fn with_config(pid: u16, config: SessionConfig) -> Result<Self, SessionError> {
        if !SESSION_PIDS.contains(&pid) {
            return Err(SessionError::UnsupportedPid(pid));
        }
        Ok(Self {
            pid,
            config,
            status: SessionStatus::Disconnected,
            deadline: None,
            outgoing: None,
            version: None,
            snapshot: None,
            pending: None,
            sync_reports: 0,
            received: VecDeque::new(),
            dropped_reports: 0,
        })
    }
    pub fn status(&self) -> &SessionStatus {
        &self.status
    }
    pub fn snapshot(&self) -> Option<&PresetSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn version(&self) -> Option<&DeviceVersion> {
        self.version.as_ref()
    }
    pub fn dropped_reports(&self) -> u64 {
        self.dropped_reports
    }
    pub fn take_received(&mut self) -> Option<Report64> {
        self.received.pop_front()
    }
    pub fn take_outgoing(&mut self) -> Option<Report64> {
        self.outgoing.take()
    }

    pub fn start(&mut self, now_ms: u64) -> Result<(), SessionError> {
        if !matches!(self.status, SessionStatus::Disconnected) {
            return Err(SessionError::Busy);
        }
        self.received.clear();
        self.dropped_reports = 0;
        self.status = SessionStatus::Detecting;
        self.queue(
            Report64::device_detect(),
            now_ms,
            self.config.detect_timeout_ms,
        );
        Ok(())
    }
    pub fn request_sync(&mut self, now_ms: u64) -> Result<(), SessionError> {
        if self.status != SessionStatus::Ready {
            return Err(SessionError::Busy);
        }
        self.begin_sync(now_ms);
        Ok(())
    }
    pub fn disconnect(&mut self) {
        self.status = SessionStatus::Disconnected;
        self.deadline = None;
        self.outgoing = None;
        self.pending = None;
        self.snapshot = None;
        self.version = None;
        self.sync_reports = 0;
    }
    pub fn fail_transport(&mut self, error: impl Into<String>) {
        self.fail(SessionError::Transport(error.into()));
    }
    /// Malformed-length input terminates active work; it cannot be padded into
    /// an ACK or a complete frame. Exact raw bytes can be logged by the worker.
    pub fn on_bytes(&mut self, bytes: &[u8], now_ms: u64) -> Result<(), SessionError> {
        let report =
            Report64::try_from(bytes).map_err(|_| SessionError::MalformedReport(bytes.len()));
        match report {
            Ok(report) => {
                self.on_report(report, now_ms);
                Ok(())
            }
            Err(error) => {
                self.fail(error.clone());
                Err(error)
            }
        }
    }
    pub fn tick(&mut self, now_ms: u64) {
        if self.deadline.is_some_and(|deadline| now_ms >= deadline) {
            self.fail(SessionError::Timeout);
        }
    }
    pub fn on_report(&mut self, report: Report64, now_ms: u64) {
        self.remember(report);
        self.tick(now_ms);
        // A response cannot complete a command the worker has not taken yet.
        if self.outgoing.is_some() {
            return;
        }
        let b = report.as_bytes();
        match self.status {
            SessionStatus::Detecting if b[0] == 0 && b[2] == 0 => {
                if b[3] != 0 {
                    self.fail(SessionError::NegativeAck(b[3]));
                    return;
                }
                self.status = SessionStatus::Version;
                self.queue(
                    Report64::version_request(),
                    now_ms,
                    self.config.version_timeout_ms,
                );
            }
            SessionStatus::Version if b[0] == 1 => {
                self.version = Some(DeviceVersion {
                    major: b[2],
                    minor: b[3],
                    build: u16::from_le_bytes([b[20], b[21]]),
                    build_date: b[4..20].try_into().expect("fixed report"),
                    raw: report,
                });
                if (self.pid == 4 && (b[2], b[3]) < (1, 2)) || (self.pid == 5 && b[2] < 1) {
                    self.fail(SessionError::UnsupportedFirmware);
                } else {
                    self.begin_sync(now_ms);
                }
            }
            SessionStatus::Syncing => self.collect(report, now_ms),
            _ => {}
        }
    }
    fn queue(&mut self, report: Report64, now_ms: u64, timeout: u64) {
        self.outgoing = Some(report);
        self.deadline = Some(now_ms.saturating_add(timeout));
    }
    fn begin_sync(&mut self, now_ms: u64) {
        self.status = SessionStatus::Syncing;
        self.pending = None;
        self.sync_reports = 0;
        self.queue(
            messages::sync_request(true),
            now_ms,
            self.config.sync_timeout_ms,
        );
    }
    fn fail(&mut self, error: SessionError) {
        self.status = SessionStatus::Failed(error);
        self.deadline = None;
        self.outgoing = None;
        self.pending = None;
    }
    fn remember(&mut self, report: Report64) {
        if self.config.max_received_reports == 0 {
            self.dropped_reports = self.dropped_reports.saturating_add(1);
            return;
        }
        if self.received.len() >= self.config.max_received_reports {
            self.received.pop_front();
            self.dropped_reports = self.dropped_reports.saturating_add(1);
        }
        self.received.push_back(report);
    }
    fn collect(&mut self, report: Report64, now_ms: u64) {
        self.sync_reports += 1;
        if self.sync_reports > self.config.max_sync_reports {
            self.fail(SessionError::SyncOverflow);
            return;
        }
        let b = report.as_bytes();
        if b[0] == 0xff {
            let Some(mut pending) = self
                .pending
                .take()
                .filter(|p| p.complete && p.snapshot.modules.len() == 5)
            else {
                self.fail(SessionError::IncompletePreset);
                return;
            };
            pending.snapshot.captured_at_ms = now_ms;
            pending
                .snapshot
                .modules
                .sort_by_key(|module| module.data_type);
            self.snapshot = Some(pending.snapshot);
            self.status = SessionStatus::Ready;
            self.deadline = None;
            return;
        }
        if b[0] != 0x1c || b[3] != 0 {
            return;
        }
        if b[2] == 4 {
            if b[6] == 0 {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| !pending.complete)
                {
                    self.fail(SessionError::InterleavedPreset);
                }
                return;
            }
            if self.pending.is_some() {
                self.fail(SessionError::DuplicateCurrentPreset);
                return;
            }
            let name: String = b[16..]
                .iter()
                .take_while(|&&byte| byte != 0)
                .map(|&byte| char::from(byte))
                .collect();
            self.pending = Some(PendingPreset {
                snapshot: PresetSnapshot {
                    name: name.trim_end_matches(' ').into(),
                    position: u16::from_le_bytes([b[4], b[5]]),
                    modified: b[7] != 0,
                    modules: Vec::new(),
                    reports: vec![report],
                    captured_at_ms: 0,
                },
                complete: false,
            });
            return;
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        if pending.complete {
            return;
        }
        pending.snapshot.reports.push(report);
        match b[2] {
            0 => {
                pending.complete = true;
                pending.snapshot.modified = b[7] != 0;
            }
            5..=9 => {
                if pending
                    .snapshot
                    .modules
                    .iter()
                    .any(|module| module.data_type == b[2])
                {
                    self.fail(SessionError::DuplicateModule(b[2]));
                    return;
                }
                pending.snapshot.modules.push(WireModule {
                    data_type: b[2],
                    model_id: u16::from_le_bytes([b[16], b[17]]),
                    raw: report,
                });
            }
            _ => {}
        }
    }
}
