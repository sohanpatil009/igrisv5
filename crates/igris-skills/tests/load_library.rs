//! Guard test: every manifest in `skills/` must load through the real
//! registry validator. Catches schema drift in the JARVIS library.

use igris_skills::SkillRegistry;
use std::fs;

fn library_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("skills")
}

#[test]
fn skill_library_loads() {
    let dir = library_dir();
    let mut registry = SkillRegistry::new();
    let mut count = 0;
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("skills/ readable")
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let json =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        registry
            .register_json(&json)
            .unwrap_or_else(|e| panic!("invalid skill {}: {e}", path.display()));
        count += 1;
    }
    assert!(count >= 18, "expected the full JARVIS pack, found {count}");
    // Every loaded skill carries explicit cautions.
    for s in registry.list() {
        assert!(
            !s.manifest.cautions.is_empty(),
            "{} has no cautions",
            s.manifest.name
        );
    }
    // High-risk skills are approval-gated.
    let gated: Vec<String> = registry
        .approval_gated()
        .iter()
        .map(|s| s.manifest.name.clone())
        .collect();
    assert!(gated.contains(&"deploy-service".to_string()));
    assert!(gated.contains(&"restart-service".to_string()));
    // Promotion + enable/disable still behave with a full registry.
    assert!(registry.disable("deploy-service").is_ok());
    assert!(!registry
        .approval_gated()
        .iter()
        .any(|s| s.manifest.name == "deploy-service"));
}
