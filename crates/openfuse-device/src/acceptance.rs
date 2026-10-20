//! Owner observations stay distinct from reproducible software replay results.
use crate::trace::{self, Origin, Trace};
use serde::{Deserialize, Serialize};

pub const CHECKS: [(&str, &str, &str); 8] = [
    (
        "read",
        "Read current preset",
        "Compare model, parameters and slot with the amplifier and its known sound.",
    ),
    (
        "live",
        "Live edits and controls",
        "Change a parameter, model, bypass and routing; check the amplifier response and readback.",
    ),
    (
        "sound",
        "Audible result",
        "Listen through the amplifier. Record the input, outputs and differences you checked.",
    ),
    (
        "save",
        "Persistent slot save",
        "Back up first. Explicitly save to a chosen test slot and verify stored readback.",
    ),
    (
        "power_cycle",
        "Survives power cycle",
        "Disconnect, switch the amplifier off/on, reconnect and verify the chosen saved slot and sound.",
    ),
    (
        "restore",
        "Backup and restore",
        "Restore the backed-up test slot, verify its readback and listen. Record archive and slot identifiers.",
    ),
    (
        "physical",
        "Physical controls and tuner",
        "Check knobs, pedal, tuner and relevant global settings on this exact amplifier.",
    ),
    (
        "reconnect",
        "Cable loss and reconnect",
        "Disconnect USB during a temporary operation; verify failure reporting and read-only reconnect without replay.",
    ),
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Pending,
    Pass,
    Fail,
    NotApplicable,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Pass => "Owner reports pass",
            Self::Fail => "Owner reports failure",
            Self::NotApplicable => "Not applicable (owner)",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub outcome: Outcome,
    pub notes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub schema: u32,
    pub trace_sha256: String,
    pub trace: Trace,
    pub amplifier_label: String,
    pub environment: String,
    pub observer: String,
    pub observations: Vec<Observation>,
}
impl Acceptance {
    pub fn new(trace: Trace) -> Result<Self, String> {
        Ok(Self {
            schema: 1,
            trace_sha256: trace.digest()?,
            trace,
            amplifier_label: String::new(),
            environment: String::new(),
            observer: String::new(),
            observations: CHECKS
                .iter()
                .map(|(id, _, _)| Observation {
                    id: (*id).into(),
                    outcome: Outcome::Pending,
                    notes: String::new(),
                })
                .collect(),
        })
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > trace::MAX_FILE_BYTES {
            return Err("Acceptance file exceeds 16 MiB".into());
        }
        let value: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.trace_sha256 != self.trace.digest()? {
            return Err("Unsupported acceptance schema or trace digest mismatch".into());
        }
        if self.observations.len() != CHECKS.len() {
            return Err("Incomplete acceptance checklist".into());
        }
        if [&self.amplifier_label, &self.environment, &self.observer]
            .iter()
            .any(|s| s.len() > 4096)
        {
            return Err("Profile text exceeds 4096 bytes".into());
        }
        let profiles: Vec<_> = self
            .trace
            .entries
            .iter()
            .filter_map(|entry| match &entry.event {
                trace::Event::Open { profile } => Some(profile),
                _ => None,
            })
            .collect();
        let one_profile = profiles
            .first()
            .is_some_and(|first| profiles.iter().all(|p| p == first));
        for (observation, (id, _, _)) in self.observations.iter().zip(CHECKS) {
            if observation.id != id || observation.notes.len() > 4096 {
                return Err("Invalid observation ID, order or note length".into());
            }
            if observation.outcome != Outcome::Pending
                && (observation.notes.trim().is_empty()
                    || self.amplifier_label.trim().is_empty()
                    || self.environment.trim().is_empty()
                    || self.observer.trim().is_empty())
            {
                return Err(
                    "Completed observations require model/revision, OS/setup, observer and a note"
                        .into(),
                );
            }
            if observation.outcome == Outcome::Pass
                && (self.trace.origin == Origin::Simulator
                    || self.trace.dropped_events != 0
                    || !one_profile)
            {
                return Err(
                    "Simulator, missing/mixed device profiles or truncated traces cannot accompany an owner pass".into(),
                );
            }
        }
        Ok(())
    }
    pub fn json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec_pretty(self).map_err(|e| e.to_string())
    }
    pub fn html(&self) -> Result<String, String> {
        self.validate()?;
        let replay = match trace::replay_bootstrap(&self.trace) {
            Ok(r) => format!(
                "Bootstrap replay passed · USB {:04X} · firmware {} · {} TX verified · {} RX consumed · {} later events not replayed",
                r.product_id, r.firmware, r.tx_verified, r.rx_consumed, r.events_not_replayed
            ),
            Err(error) => format!("Bootstrap replay did not pass: {error}"),
        };
        let mut rows = String::new();
        for (observation, (_, title, instruction)) in self.observations.iter().zip(CHECKS) {
            rows.push_str(&format!(
                "<tr><td><strong>{}</strong><p>{}</p></td><td>{}</td><td class='note'>{}</td></tr>",
                escape(title),
                escape(instruction),
                observation.outcome.label(),
                escape(&observation.notes)
            ));
        }
        let profiles = self
            .trace
            .entries
            .iter()
            .filter_map(|entry| match &entry.event {
                trace::Event::Open { profile } | trace::Event::OpenFailed { profile } => {
                    Some(format!(
                        "USB {:04X} / interface {}",
                        profile.product_id, profile.interface_number
                    ))
                }
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        let origin = match self.trace.origin {
            Origin::Simulator => "SIMULATOR · NO HARDWARE EVIDENCE",
            Origin::HardwareTransport => "HARDWARE TRANSPORT · OWNER OBSERVATIONS REQUIRE REVIEW",
        };
        Ok(format!(
            r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>OpenFUSE acceptance session</title><style>body{{margin:0;background:#171b1c;color:#eee9dc;font:16px system-ui;line-height:1.55}}main{{max-width:1080px;margin:auto;padding:40px 24px}}h1{{font-size:38px}}small,p{{color:#c0c5bf}}.badge{{color:#efbd73}}section{{border:1px solid #46504b;border-radius:14px;padding:20px;margin:20px 0}}table{{border-collapse:collapse;width:100%}}td,th{{padding:16px;text-align:left;border-bottom:1px solid #46504b;vertical-align:top}}td:first-child{{width:35%}}.note{{white-space:pre-wrap;overflow-wrap:anywhere}}code{{overflow-wrap:anywhere}}@media(max-width:700px){{td,th{{display:block;padding:8px}}td:first-child{{width:auto}}}}</style><main><div class="badge">{origin}</div><h1>Amplifier acceptance session</h1><p>Model / revision: {}<br>OS version, USB setup and outputs: {}<br>Observer: {}</p><section><strong>{}</strong><p>Replay executes the production read-session state machine. It verifies the initial detect/version/current-preset exchange only. Sound, persistent memory, later writes and reconnect behavior are separate owner observations below. No overall hardware certification is inferred.</p></section><table><thead><tr><th>Check</th><th>Owner observation</th><th>Evidence / notes</th></tr></thead><tbody>{rows}</tbody></table><section><p>Attempted device profiles: {profiles}</p><p>Trace origin: {:?} · {} / {} · app {} · started Unix ms {} · {} events · {} dropped</p><code>SHA-256 {}</code><p>Digest binds this report to the canonical trace in its JSON bundle; it is not an authenticity signature. Device paths and serial numbers are omitted; raw preset payloads and owner notes may still be private. Normalized 64-byte reports only: native open/drain, malformed buffers, USB audio and idle reads are outside the recording boundary.</p></section></main></html>"#,
            escape(&self.amplifier_label),
            escape(&self.environment),
            escape(&self.observer),
            escape(&replay),
            self.trace.origin,
            escape(&self.trace.os),
            escape(&self.trace.architecture),
            escape(&self.trace.app_version),
            self.trace.started_unix_ms,
            self.trace.entries.len(),
            self.trace.dropped_events,
            self.trace_sha256
        ))
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
