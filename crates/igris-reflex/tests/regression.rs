//! Guard test: the regression corpus in `tests/regression_corpus.json`
//! pins reflex behavior. Any intent/routing change that breaks a case must
//! update the corpus deliberately — never silently.

use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct Case {
    input: String,
    intent: String,
    requires_reasoning: bool,
    approval: bool,
}

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

fn intent_key(intent: igris_reflex::Intent) -> &'static str {
    match intent {
        igris_reflex::Intent::DeviceTransfer => "DeviceTransfer",
        igris_reflex::Intent::DeviceStatus => "DeviceStatus",
        igris_reflex::Intent::MemoryQuery => "MemoryQuery",
        igris_reflex::Intent::ToolExec => "ToolExec",
        igris_reflex::Intent::Communication => "Communication",
        igris_reflex::Intent::RiskyOp => "RiskyOp",
        igris_reflex::Intent::General => "General",
    }
}

#[test]
fn regression_corpus_holds() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("regression_corpus.json");
    let json = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let corpus: Corpus = serde_json::from_str(&json).expect("valid corpus");
    assert!(!corpus.cases.is_empty(), "corpus must not shrink to zero");
    let mut failures = Vec::new();
    for case in &corpus.cases {
        let d = igris_reflex::decide(&case.input);
        if intent_key(d.intent) != case.intent
            || d.requires_reasoning != case.requires_reasoning
            || d.approval != case.approval
        {
            failures.push(format!(
                "{:?}: got {:?}/reasoning={}/approval={}",
                case.input, d.intent, d.requires_reasoning, d.approval
            ));
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
