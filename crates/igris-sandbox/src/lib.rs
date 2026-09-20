//! igris-sandbox — workspace enforcement + secret isolation.
//!
//! Phase 8: every agent execution is confined to a `Workspace` (allowed
//! file roots, command denylist, network flag, env allowlist, timeout and
//! output caps), and credentials live in a `SecretsVault` that never leaks
//! values into logs — `redact()` scrubs them from any text.
//! OS-level isolation (containers, user namespaces) is hardening work;
//! the boundary contract is established here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, thiserror::Error)]
pub enum SandboxError {
    #[error("path escape: {0}")]
    PathEscape(String),
    #[error("command denied: {0}")]
    CommandDenied(String),
    #[error("network disabled for this workspace")]
    NetworkDisabled,
    #[error("unknown secret: {0}")]
    UnknownSecret(String),
    #[error("duplicate secret: {0}")]
    DuplicateSecret(String),
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    /// Canonical file roots the agent may touch.
    pub allowed_roots: Vec<PathBuf>,
    /// Lowercase substrings; any command containing one is refused.
    pub denied_commands: Vec<String>,
    pub network_allowed: bool,
    /// Only these env vars are visible inside the workspace.
    pub allowed_env: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_chars: usize,
}

impl Workspace {
    pub fn new(name: &str, allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            name: name.into(),
            allowed_roots,
            denied_commands: vec![
                "rm -rf /".into(),
                "mkfs".into(),
                "dd if=".into(),
                ":(){".into(),
                "shutdown".into(),
                "> /dev/sda".into(),
            ],
            network_allowed: false,
            allowed_env: vec!["PATH".into()],
            timeout_ms: 30_000,
            max_output_chars: 8_192,
        }
    }

    /// Lexical confinement: rejects absolute paths and `..`/prefix escapes,
    /// then requires the joined path to sit under a canonical root.
    /// (Symlink-race hardening arrives with OS isolation.)
    pub fn confine(&self, rel: &str) -> Result<PathBuf, SandboxError> {
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err(SandboxError::PathEscape(rel.into()));
        }
        if p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(SandboxError::PathEscape(rel.into()));
        }
        for root in &self.allowed_roots {
            let joined = root.join(p);
            if joined.starts_with(root) {
                return Ok(joined);
            }
        }
        Err(SandboxError::PathEscape(format!(
            "outside workspace roots: {rel}"
        )))
    }

    pub fn check_command(&self, cmd: &str) -> Result<(), SandboxError> {
        let lower = cmd.to_lowercase();
        if self
            .denied_commands
            .iter()
            .any(|d| lower.contains(&d.to_lowercase()))
        {
            return Err(SandboxError::CommandDenied(cmd.into()));
        }
        Ok(())
    }

    pub fn check_network(&self, needs_network: bool) -> Result<(), SandboxError> {
        if needs_network && !self.network_allowed {
            return Err(SandboxError::NetworkDisabled);
        }
        Ok(())
    }

    /// Strip the process env down to the allowlist.
    pub fn filter_env(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        env.iter()
            .filter(|(k, _)| self.allowed_env.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn cap_output(&self, s: &str) -> String {
        if s.len() <= self.max_output_chars {
            s.to_string()
        } else {
            format!("{}…[truncated]", &s[..self.max_output_chars])
        }
    }
}

// ---------------------------------------------------------------------------
// Secrets vault — values in, redacted text out. Never logged.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SecretsVault {
    secrets: HashMap<String, String>,
}

impl SecretsVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), SandboxError> {
        if key.trim().is_empty() || value.is_empty() {
            return Err(SandboxError::UnknownSecret("empty key or value".into()));
        }
        if self.secrets.contains_key(key) {
            return Err(SandboxError::DuplicateSecret(key.into()));
        }
        self.secrets.insert(key.into(), value.into());
        Ok(())
    }

    pub fn has(&self, key: &str) -> bool {
        self.secrets.contains_key(key)
    }

    /// Authorized read. Callers must never log or embed the result in prompts.
    pub fn reveal(&self, key: &str) -> Result<&str, SandboxError> {
        self.secrets
            .get(key)
            .map(|s| s.as_str())
            .ok_or_else(|| SandboxError::UnknownSecret(key.into()))
    }

    /// Scrub every known value from text (logs, tool output, error strings).
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        let mut values: Vec<&String> = self.secrets.values().collect();
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        for v in values {
            if v.len() >= 4 {
                out = out.replace(v, "[redacted]");
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        Workspace::new("test-ws", vec![PathBuf::from("/tmp/igris-ws")])
    }

    #[test]
    fn paths_confined() {
        let w = ws();
        assert!(w.confine("notes/a.txt").is_ok());
        assert!(w.confine("../evil").is_err());
        assert!(w.confine("/etc/passwd").is_err());
        // Empty roots confine nothing.
        let bare = Workspace::new("bare", vec![]);
        assert!(bare.confine("a.txt").is_err());
    }

    #[test]
    fn commands_denied() {
        let w = ws();
        assert!(w.check_command("echo hello").is_ok());
        assert!(matches!(
            w.check_command("sudo rm -rf / tmp"),
            Err(SandboxError::CommandDenied(_))
        ));
        assert!(matches!(
            w.check_command("MKFS /dev/sda"),
            Err(SandboxError::CommandDenied(_))
        ));
    }

    #[test]
    fn network_gated() {
        let mut w = ws();
        assert!(w.check_network(false).is_ok());
        assert!(matches!(
            w.check_network(true),
            Err(SandboxError::NetworkDisabled)
        ));
        w.network_allowed = true;
        assert!(w.check_network(true).is_ok());
    }

    #[test]
    fn env_filtered_and_output_capped() {
        let w = ws();
        let mut env = HashMap::new();
        env.insert("PATH".into(), "/bin".into());
        env.insert("GROQ_API_KEY".into(), "secret".into());
        let filtered = w.filter_env(&env);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("PATH"));
        let long = "x".repeat(w.max_output_chars + 10);
        assert!(w.cap_output(&long).ends_with("[truncated]"));
    }

    #[test]
    fn vault_isolation_and_redaction() {
        let mut v = SecretsVault::new();
        v.set("GROQ_API_KEY", "gsk-super-secret-value").unwrap();
        assert!(v.has("GROQ_API_KEY"));
        assert_eq!(v.reveal("GROQ_API_KEY").unwrap(), "gsk-super-secret-value");
        assert!(v.set("GROQ_API_KEY", "again").is_err());
        assert!(v.reveal("MISSING").is_err());
        // Redaction scrubs values from arbitrary text (logs, errors, output).
        let dirty = "call failed with key gsk-super-secret-value in output";
        let clean = v.redact(dirty);
        assert!(!clean.contains("gsk-super-secret-value"));
        assert!(clean.contains("[redacted]"));
        // Short values (<4 chars) are skipped to avoid over-redaction.
        v.set("PIN", "123").unwrap();
        assert!(v.redact("pin 123 here").contains("123"));
    }

    #[test]
    fn failure_injection_unknowns_are_safe() {
        // Unknown workspace roots, empty vaults, and empty commands all fail closed.
        let w = Workspace::new("empty", vec![]);
        assert!(w.confine("x").is_err());
        let mut v = SecretsVault::new();
        assert!(v.redact("nothing to hide").contains("nothing"));
        assert!(v.set("", "x").is_err());
    }
}
