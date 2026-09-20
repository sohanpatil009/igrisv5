//! Failure-injection across the security chain:
//! policy gate -> approval -> sandbox workspace -> tool -> secret redaction.
//! Every injected failure must fail closed (deny, never silently allow).

use chrono::TimeDelta;
use igris_policy::{Autonomy, PermissionLevel, Policy};
use igris_sandbox::{SecretsVault, Workspace};
use igris_tools::{FilesystemTool, Registry, TerminalTool};
use std::sync::Arc;

fn workspace() -> Workspace {
    let root = std::env::temp_dir().join("igris-chain-test");
    std::fs::create_dir_all(&root).unwrap();
    Workspace::new("chain", vec![root])
}

#[test]
fn destructive_command_fails_at_every_layer() {
    let policy = Policy {
        autonomy: Autonomy::Autonomous,
        max_auto_level: PermissionLevel::L4Destructive,
        ..Default::default()
    };
    let ws = workspace();
    let registry = Registry::new();
    registry.register(Arc::new(TerminalTool::new()));

    // Layer 1 — sandbox refuses the command shape.
    assert!(ws.check_command("rm -rf / tmp").is_err());
    // Layer 2 — the tool itself refuses it (defense in depth, independent lists).
    let out = registry
        .execute("terminal", r#"{"cmd":"rm -rf / tmp"}"#)
        .unwrap();
    assert!(!out.ok);
    // Layer 3 — policy still gates the permission even in autonomous mode
    // once the kill-switch engages (injected failure: operator hits kill).
    policy.kill.engage();
    assert!(policy.check(PermissionLevel::L0Read, false).is_err());
}

#[test]
fn path_escape_blocked_before_tool_runs() {
    let ws = workspace();
    assert!(ws.confine("../outside.txt").is_err());

    let root = std::env::temp_dir().join("igris-chain-test");
    let registry = Registry::new();
    registry.register(Arc::new(FilesystemTool::new(root).unwrap()));
    let out = registry
        .execute("filesystem", r#"{"op":"read","path":"../../outside.txt"}"#)
        .unwrap();
    assert!(!out.ok);
}

#[test]
fn approval_expiry_denies_stale_high_risk_work() {
    use igris_policy::ApprovalStore;
    let policy = Policy::default();
    let mut store = ApprovalStore::new();
    // Injected failure: approval arrives but the operator is too slow.
    let id = store.request(
        "restart staging",
        PermissionLevel::L3Sensitive,
        TimeDelta::milliseconds(1),
    );
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(policy
        .check_with_approval(PermissionLevel::L3Sensitive, true, &mut store, &id)
        .is_err());
}

#[test]
fn secrets_never_reach_logs_even_on_tool_error() {
    let mut vault = SecretsVault::new();
    vault.set("DEPLOY_TOKEN", "tok-live-abcdef-123456").unwrap();
    let registry = Registry::new();
    registry.register(Arc::new(TerminalTool::new()));
    // Failing command whose output echoes the secret back.
    let out = registry
        .execute(
            "terminal",
            r#"{"cmd":"echo tok-live-abcdef-123456 && exit 3"}"#,
        )
        .unwrap();
    assert!(!out.ok);
    let logged = format!("tool failed: {out:?}");
    let scrubbed = vault.redact(&logged);
    assert!(!scrubbed.contains("tok-live-abcdef-123456"));
}

#[test]
fn network_tool_refused_in_offline_workspace() {
    use igris_tools::BrowserTool;
    let ws = workspace();
    assert!(!ws.network_allowed);
    assert!(ws.check_network(true).is_err());
    // Browser def is honest about transport; the workspace gate is independent.
    let registry = Registry::new();
    registry.register(Arc::new(BrowserTool));
    let def = &registry.definitions_for(&["browser".to_string()])[0];
    assert!(def.requires_network);
}
