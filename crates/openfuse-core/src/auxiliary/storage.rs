use super::*;
use sha2::{Digest, Sha256};
pub const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: String,
    version: u32,
    pid: u16,
    reports: Vec<String>,
    sha256: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn checksum(file: &File) -> Result<String> {
    Ok(digest(
        &serde_json::to_vec(&(&file.format, file.version, file.pid, &file.reports))
            .map_err(error)?,
    ))
}
pub(super) fn serialize(snapshot: &Snapshot) -> Result<String> {
    if snapshot.legacy {
        return Err(error(
            "Legacy XML-only sources retain their original folder archive; no raw capture is fabricated",
        ));
    }
    let mut file = File {
        format: "confuse-auxiliary".into(),
        version: 1,
        pid: snapshot.pid,
        reports: snapshot
            .raw
            .iter()
            .map(|r| crate::hex::encode(r.as_bytes()))
            .collect(),
        sha256: String::new(),
    };
    file.sha256 = checksum(&file)?;
    let source = serde_json::to_string_pretty(&file).map_err(error)?;
    if source.len() > MAX_SNAPSHOT_BYTES {
        return Err(error("Auxiliary capture exceeds 4 MiB"));
    }
    Ok(source)
}
pub(super) fn parse(source: &str) -> Result<Snapshot> {
    if source.len() > MAX_SNAPSHOT_BYTES {
        return Err(error("Auxiliary capture exceeds 4 MiB"));
    }
    let file: File = serde_json::from_str(source).map_err(error)?;
    if file.format != "confuse-auxiliary"
        || file.version != 1
        || file.sha256 != checksum(&file)?
        || file.reports.len() > collect::MAX_REPORTS
    {
        return Err(error("Unsupported or corrupt auxiliary archive"));
    }
    let reports = file
        .reports
        .iter()
        .map(|hex| {
            if hex.len() > 191 {
                return Err(error("Auxiliary report exceeds 64 bytes"));
            }
            let raw = crate::hex::decode(hex).map_err(error)?;
            Report64::try_from(raw.as_slice()).map_err(error)
        })
        .collect::<Result<Vec<_>>>()?;
    Snapshot::from_reports(file.pid, &reports)
}
pub(super) fn canonical(reports: &[Report64]) -> Vec<(u8, Vec<u8>)> {
    let mut values = reports
        .iter()
        .filter(|r| r.message_id() == 0x1c)
        .map(|r| {
            let b = r.as_bytes();
            // Ignore known transient transport/current/modified bits, but retain
            // unfamiliar header bits and all payload bytes, including Complete.
            let mut data = vec![b[1] & !0xc7, b[6] & !1, b[7] & !1];
            data.extend_from_slice(&b[8..]);
            (b[2], data)
        })
        .collect::<Vec<_>>();
    values.sort();
    values
}
pub(super) fn fingerprint(snapshot: &Snapshot) -> String {
    let auxiliary = snapshot
        .entries
        .iter()
        .map(|e| (e.key, canonical(&e.raw)))
        .collect::<Vec<_>>();
    let presets = snapshot
        .presets
        .iter()
        .map(|e| (e.slot(), canonical(e.raw_reports())))
        .collect::<Vec<_>>();
    digest(&serde_json::to_vec(&(snapshot.pid, auxiliary, presets)).expect("serializable capture"))
}
impl Serialize for Snapshot {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let text = self.to_json().map_err(serde::ser::Error::custom)?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}
