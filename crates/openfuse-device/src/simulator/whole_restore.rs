//! Synthetic whole-restore peer. Stored endpoint bytes follow written preset
//! packets; post-System propagation by physical firmware remains unverified.
use super::*;
impl Peer {
    pub(super) fn restore_boundary(&mut self, report: Report64) -> Result<bool, String> {
        let b = report.as_bytes();
        if b[3] == 0 {
            if self.whole_restore {
                return Err("Read during whole restore envelope".into());
            }
            return Ok(false);
        }
        let flags = match b[3] {
            1 => 0xc3,
            2 => 3,
            _ => return Err("Unknown restore boundary".into()),
        };
        if b[1] != flags
            || b[2] != 0
            || b[4] != 1
            || b[5..].iter().any(|v| *v != 0)
            || self.staged.is_some()
            || self.save.is_some()
            || self.staged_auxiliary.is_some()
            || (b[3] == 1) == self.whole_restore
        {
            return Err("Invalid or nested whole restore envelope".into());
        }
        self.whole_restore = b[3] == 1;
        self.ack(0x14);
        Ok(true)
    }
    pub(super) fn restore_system(&mut self, report: Report64) -> Result<(), String> {
        let b = report.as_bytes();
        if !self.whole_restore
            || self.pid == 14
            || b[1] != 1
            || b[3..6] != [0; 3]
            || b[6..8] != [1; 2]
            || b[8..16].iter().chain(&b[38..]).any(|v| *v != 0)
            || self.staged.is_some()
            || self.save.is_some()
        {
            return Err("System write requires a valid whole restore envelope".into());
        }
        let mut raw = *b;
        raw[6] = 0;
        raw[7] = 0;
        self.auxiliary
            .lock()
            .unwrap()
            .insert((0, 0), vec![Report64::from_bytes(raw)]);
        self.globals = (b[35], b[36]);
        Ok(())
    }
}
