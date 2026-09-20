//! igris-skills — persistent procedural knowledge.
//!
//! Skills are versioned, permissioned runbooks. Repeated successful
//! workflows graduate via `promotion_candidates()`; nothing self-installs,
//! and disabled skills never execute.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const KNOWN_PERMISSIONS: [&str; 5] = ["L0", "L1", "L2", "L3", "L4"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub inputs: Vec<String>,
    pub preconditions: Vec<String>,
    pub permissions: Vec<String>,
    pub tools: Vec<String>,
    pub steps: Vec<String>,
    pub validation: String,
    pub failure_recovery: String,
    /// Safety notes the assistant must state or check before running.
    /// Defaults empty so older manifests still parse.
    #[serde(default)]
    pub cautions: Vec<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SkillError {
    #[error("invalid skill {name}: {reason}")]
    Invalid { name: String, reason: String },
    #[error("duplicate skill: {0}")]
    Duplicate(String),
    #[error("unknown skill: {0}")]
    Unknown(String),
}

impl SkillManifest {
    pub fn validate(&self) -> Result<(), SkillError> {
        let bad = |reason: &str| SkillError::Invalid {
            name: self.name.clone(),
            reason: reason.into(),
        };
        if self.name.trim().is_empty() {
            return Err(bad("name is empty"));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(bad("name must be lowercase kebab-case"));
        }
        if self.description.trim().is_empty() {
            return Err(bad("description is empty"));
        }
        if self.version.trim().is_empty() {
            return Err(bad("version is empty"));
        }
        if self.tools.is_empty() {
            return Err(bad("tools is empty"));
        }
        if self.steps.is_empty() {
            return Err(bad("steps is empty"));
        }
        for p in &self.permissions {
            if !KNOWN_PERMISSIONS.contains(&p.as_str()) {
                return Err(bad(&format!("unknown permission {p} (want L0..L4)")));
            }
        }
        if self.validation.trim().is_empty() {
            return Err(bad("validation is empty"));
        }
        Ok(())
    }

    /// Highest permission level this skill touches.
    pub fn max_permission(&self) -> &str {
        self.permissions
            .iter()
            .max()
            .map(|s| s.as_str())
            .unwrap_or("L0")
    }

    /// L3+ skills must stop for user approval before executing.
    /// Mirrors `igris-policy`: sensitive and destructive work is never silent.
    pub fn requires_approval(&self) -> bool {
        matches!(self.max_permission(), "L3" | "L4")
    }
}

#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub manifest: SkillManifest,
    pub enabled: bool,
    pub uses: u64,
    pub successes: u64,
}

#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: HashMap<String, SkillRecord>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: SkillManifest) -> Result<(), SkillError> {
        manifest.validate()?;
        if self.skills.contains_key(&manifest.name) {
            return Err(SkillError::Duplicate(manifest.name));
        }
        let name = manifest.name.clone();
        self.skills.insert(
            name,
            SkillRecord {
                manifest,
                enabled: true,
                uses: 0,
                successes: 0,
            },
        );
        Ok(())
    }

    pub fn register_json(&mut self, json: &str) -> Result<String, SkillError> {
        let manifest: SkillManifest =
            serde_json::from_str(json).map_err(|e| SkillError::Invalid {
                name: "?".into(),
                reason: e.to_string(),
            })?;
        let name = manifest.name.clone();
        self.register(manifest)?;
        Ok(name)
    }

    pub fn get(&self, name: &str) -> Option<&SkillRecord> {
        self.skills.get(name)
    }

    pub fn enable(&mut self, name: &str) -> Result<(), SkillError> {
        self.skills
            .get_mut(name)
            .map(|r| r.enabled = true)
            .ok_or_else(|| SkillError::Unknown(name.into()))
    }

    pub fn disable(&mut self, name: &str) -> Result<(), SkillError> {
        self.skills
            .get_mut(name)
            .map(|r| r.enabled = false)
            .ok_or_else(|| SkillError::Unknown(name.into()))
    }

    pub fn list(&self) -> Vec<&SkillRecord> {
        let mut out: Vec<&SkillRecord> = self.skills.values().collect();
        out.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        out
    }

    pub fn list_enabled(&self) -> Vec<&SkillRecord> {
        self.list().into_iter().filter(|r| r.enabled).collect()
    }

    /// Enabled skills that must stop for approval (L3+). The assistant
    /// surfaces these with their cautions before running anything.
    pub fn approval_gated(&self) -> Vec<&SkillRecord> {
        self.list_enabled()
            .into_iter()
            .filter(|r| r.manifest.requires_approval())
            .collect()
    }

    pub fn record_use(&mut self, name: &str) -> Result<(), SkillError> {
        self.skills
            .get_mut(name)
            .map(|r| r.uses += 1)
            .ok_or_else(|| SkillError::Unknown(name.into()))
    }

    pub fn record_success(&mut self, name: &str) -> Result<(), SkillError> {
        self.skills
            .get_mut(name)
            .map(|r| {
                r.uses += 1;
                r.successes += 1;
            })
            .ok_or_else(|| SkillError::Unknown(name.into()))
    }

    /// Skills with enough recorded successes to graduate into trusted defaults.
    pub fn promotion_candidates(&self, min_successes: u64) -> Vec<String> {
        let mut out: Vec<String> = self
            .skills
            .values()
            .filter(|r| r.successes >= min_successes)
            .map(|r| r.manifest.name.clone())
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> SkillManifest {
        SkillManifest {
            name: name.into(),
            description: "Prepare a morning standup brief.".into(),
            version: "0.1.0".into(),
            inputs: vec!["date".into()],
            preconditions: vec!["memory store reachable".into()],
            permissions: vec!["L0".into(), "L1".into()],
            tools: vec!["memory_search".into()],
            steps: vec!["collect yesterday's notes".into(), "draft brief".into()],
            validation: "brief contains 3 sections".into(),
            failure_recovery: "fall back to raw notes".into(),
            cautions: vec![],
        }
    }

    #[test]
    fn register_and_list() {
        let mut r = SkillRegistry::new();
        r.register(manifest("morning-brief")).unwrap();
        assert_eq!(r.list().len(), 1);
        assert_eq!(
            r.get("morning-brief").unwrap().manifest.max_permission(),
            "L1"
        );
    }

    #[test]
    fn reject_empty_tools_and_steps() {
        let mut r = SkillRegistry::new();
        let mut m = manifest("bad-skill");
        m.tools.clear();
        assert!(matches!(r.register(m), Err(SkillError::Invalid { .. })));
    }

    #[test]
    fn reject_bad_permission_and_name() {
        let mut r = SkillRegistry::new();
        let mut m = manifest("Bad_Name");
        assert!(r.register(m.clone()).is_err());
        m.name = "ok-name".into();
        m.permissions = vec!["root".into()];
        assert!(matches!(r.register(m), Err(SkillError::Invalid { .. })));
    }

    #[test]
    fn reject_duplicate() {
        let mut r = SkillRegistry::new();
        r.register(manifest("dup-skill")).unwrap();
        assert!(matches!(
            r.register(manifest("dup-skill")),
            Err(SkillError::Duplicate(_))
        ));
    }

    #[test]
    fn enable_disable() {
        let mut r = SkillRegistry::new();
        r.register(manifest("toggle-skill")).unwrap();
        r.disable("toggle-skill").unwrap();
        assert!(r.list_enabled().is_empty());
        r.enable("toggle-skill").unwrap();
        assert_eq!(r.list_enabled().len(), 1);
        assert!(r.disable("ghost").is_err());
    }

    #[test]
    fn success_promotion() {
        let mut r = SkillRegistry::new();
        r.register(manifest("rising-skill")).unwrap();
        for _ in 0..3 {
            r.record_success("rising-skill").unwrap();
        }
        assert_eq!(r.promotion_candidates(3), vec!["rising-skill".to_string()]);
        assert!(r.promotion_candidates(4).is_empty());
    }

    #[test]
    fn json_roundtrip() {
        let mut r = SkillRegistry::new();
        let json = serde_json::to_string(&manifest("json-skill")).unwrap();
        let name = r.register_json(&json).unwrap();
        assert_eq!(name, "json-skill");
        assert!(r.register_json("{not json").is_err());
    }

    #[test]
    fn legacy_manifest_without_cautions_still_parses() {
        let mut r = SkillRegistry::new();
        let json = r#"{
            "name": "legacy-skill", "description": "old skill", "version": "0.1.0",
            "inputs": [], "preconditions": [], "permissions": ["L0"],
            "tools": ["memory_search"], "steps": ["do it"],
            "validation": "done", "failure_recovery": "retry"
        }"#;
        r.register_json(json).unwrap();
        assert!(r.get("legacy-skill").unwrap().manifest.cautions.is_empty());
    }

    #[test]
    fn approval_gating_by_permission() {
        let mut r = SkillRegistry::new();
        r.register(manifest("safe-skill")).unwrap();
        let mut risky = manifest("risky-skill");
        risky.permissions = vec!["L1".into(), "L4".into()];
        risky.cautions = vec!["destructive: confirm target first".into()];
        r.register(risky).unwrap();
        assert!(!r.get("safe-skill").unwrap().manifest.requires_approval());
        assert!(r.get("risky-skill").unwrap().manifest.requires_approval());
        let gated: Vec<String> = r
            .approval_gated()
            .iter()
            .map(|s| s.manifest.name.clone())
            .collect();
        assert_eq!(gated, vec!["risky-skill".to_string()]);
        // Disabled risky skills drop out of the gate list.
        r.disable("risky-skill").unwrap();
        assert!(r.approval_gated().is_empty());
    }
}
