//! OTP pairing: 6-digit codes, 120s expiry, 3 attempts.
//! The code travels out-of-band (user reads it off the other screen);
//! the wire only ever sees the verification result.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const OTP_TTL: Duration = Duration::from_secs(120);
pub const OTP_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug)]
struct Pending {
    code: String,
    expires: Instant,
    attempts: u32,
}

#[derive(Debug, Default)]
pub struct OtpManager {
    pending: HashMap<String, Pending>,
}

impl OtpManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a code for a device. Any sender randomness source works;
    /// test hook `issue_fixed` below covers determinism.
    pub fn issue(&mut self, device_id: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(123456);
        let code = format!("{:06}", (nanos ^ 0x9e3779b9) % 1_000_000);
        self.pending.insert(
            device_id.into(),
            Pending {
                code: code.clone(),
                expires: Instant::now() + OTP_TTL,
                attempts: 0,
            },
        );
        code
    }

    #[cfg(test)]
    fn issue_fixed(&mut self, device_id: &str, code: &str) {
        self.pending.insert(
            device_id.into(),
            Pending {
                code: code.into(),
                expires: Instant::now() + OTP_TTL,
                attempts: 0,
            },
        );
    }

    pub fn verify(&mut self, device_id: &str, code: &str) -> bool {
        let Some(p) = self.pending.get_mut(device_id) else {
            return false;
        };
        if Instant::now() > p.expires {
            self.pending.remove(device_id);
            return false;
        }
        p.attempts += 1;
        if p.code == code {
            self.pending.remove(device_id);
            return true;
        }
        if p.attempts >= OTP_MAX_ATTEMPTS {
            self.pending.remove(device_id);
        }
        false
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn sweep_expired(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, p| now <= p.expires);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_verify_roundtrip() {
        let mut m = OtpManager::new();
        m.issue_fixed("d1", "123456");
        assert!(m.verify("d1", "123456"));
        assert_eq!(m.pending_count(), 0);
    }

    #[test]
    fn wrong_code_fails_and_burns_after_three() {
        let mut m = OtpManager::new();
        m.issue_fixed("d1", "123456");
        assert!(!m.verify("d1", "000000"));
        assert!(!m.verify("d1", "000001"));
        assert!(!m.verify("d1", "000002"));
        // Burned: even the right code now fails.
        assert!(!m.verify("d1", "123456"));
    }

    #[test]
    fn unknown_device_fails() {
        let mut m = OtpManager::new();
        assert!(!m.verify("ghost", "123456"));
    }

    #[test]
    fn issued_codes_are_six_digits() {
        let mut m = OtpManager::new();
        let code = m.issue("d1");
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
