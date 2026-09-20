//! igris-tools — permissioned tool runtime.
//!
//! Phase 7: real sandboxed executors (filesystem root-confined, terminal
//! denylisted), validate-before-run, registry timeouts, and a lazy catalog
//! with capability filtering so models only ever see relevant tools.
//! Browser fetch transport lands with device-mesh networking (Phase 9).

use igris_policy::PermissionLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
pub const MAX_OUTPUT_CHARS: usize = 8_192;
pub const MAX_FILE_READ_BYTES: u64 = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Medium and above stop for approval. Mirrors the skill gate (L3+).
    pub fn needs_approval(&self) -> bool {
        *self >= RiskLevel::Medium
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub version: String,
    pub permission: PermissionLevel,
    pub risk: RiskLevel,
    pub requires_network: bool,
    /// Per-execution ceiling. `None` = registry default (30s).
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(output: String) -> Self {
        Self {
            ok: true,
            output: truncate(output),
            error: None,
        }
    }

    pub fn err(msg: String) -> Self {
        Self {
            ok: false,
            output: String::new(),
            error: Some(truncate(msg)),
        }
    }
}

fn truncate(s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s
    } else {
        format!(
            "{}…[truncated {} chars]",
            &s[..MAX_OUTPUT_CHARS],
            s.len() - MAX_OUTPUT_CHARS
        )
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    #[error("denied: {0}")]
    Denied(String),
    #[error("bad args: {0}")]
    BadArgs(String),
    #[error("timed out after {0}ms")]
    Timeout(u64),
    #[error("execution failed: {0}")]
    Exec(String),
}

pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDef;
    /// Fast argument check. Runs before any side effect.
    fn validate(&self, _args: &str) -> Result<(), ToolError> {
        Ok(())
    }
    fn run(&self, args: &str) -> ToolResult;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub struct Registry {
    inner: Mutex<HashMap<String, Arc<dyn Tool>>>,
    audit: Mutex<Vec<(String, bool)>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            audit: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        self.inner
            .lock()
            .unwrap()
            .insert(tool.definition().name.clone(), tool);
    }

    pub fn has(&self, name: &str) -> bool {
        self.inner.lock().unwrap().contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.inner.lock().unwrap().keys().cloned().collect();
        out.sort();
        out
    }

    fn record(&self, name: &str, ok: bool) {
        let mut a = self.audit.lock().unwrap();
        a.push((name.into(), ok));
        let len = a.len();
        if len > 10_000 {
            a.drain(0..len - 10_000);
        }
    }

    /// Synchronous execute: validates first, then runs inline.
    /// Prefer `execute_async` for anything that can block.
    pub fn execute(&self, name: &str, args: &str) -> Option<ToolResult> {
        let t = self.inner.lock().unwrap().get(name).cloned()?;
        let r = match t.validate(args) {
            Err(e) => ToolResult::err(e.to_string()),
            Ok(()) => t.run(args),
        };
        self.record(name, r.ok);
        Some(r)
    }

    /// Async execute with timeout: validation runs inline, `run()` runs on
    /// the blocking pool so a hung tool can never stall the async runtime.
    pub async fn execute_async(&self, name: &str, args: &str) -> Option<ToolResult> {
        let t = self.inner.lock().ok()?.get(name).cloned()?;
        let timeout_ms = t.definition().timeout_ms.unwrap_or(DEFAULT_TOOL_TIMEOUT_MS);
        self.execute_inner(name, t, args.to_string(), timeout_ms)
            .await
    }

    /// Same, with an explicit ceiling (budgets, tests).
    pub async fn execute_async_with_timeout(
        &self,
        name: &str,
        args: &str,
        timeout_ms: u64,
    ) -> Option<ToolResult> {
        let t = self.inner.lock().ok()?.get(name).cloned()?;
        self.execute_inner(name, t, args.to_string(), timeout_ms)
            .await
    }

    async fn execute_inner(
        &self,
        name: &str,
        t: Arc<dyn Tool>,
        args: String,
        timeout_ms: u64,
    ) -> Option<ToolResult> {
        if let Err(e) = t.validate(&args) {
            let r = ToolResult::err(e.to_string());
            self.record(name, false);
            return Some(r);
        }
        let res = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || t.run(&args)),
        )
        .await;
        let r = match res {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => ToolResult::err(format!("tool panicked: {e}")),
            Err(_) => ToolResult::err(ToolError::Timeout(timeout_ms).to_string()),
        };
        self.record(name, r.ok);
        Some(r)
    }

    /// Capability filtering: the model only sees defs for its allowed tools.
    /// This is the contract the router/agent layer relies on — never dump
    /// the whole registry into a prompt.
    pub fn definitions_for(&self, allowed: &[String]) -> Vec<ToolDef> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<ToolDef> = allowed
            .iter()
            .filter_map(|n| inner.get(n).map(|t| t.definition()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn success_rate(&self, name: &str) -> Option<f32> {
        let a = self.audit.lock().unwrap();
        let mine: Vec<bool> = a
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, ok)| *ok)
            .collect();
        if mine.is_empty() {
            return None;
        }
        Some(mine.iter().filter(|&&x| x).count() as f32 / mine.len() as f32)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Lazy catalog (MCP-style discovery seed)
// ---------------------------------------------------------------------------

/// Lazy constructor for a tool. Factories are consumed on first use.
pub type ToolFactory = Box<dyn Fn() -> Arc<dyn Tool> + Send + Sync>;

/// Tools are described up front but instantiated on first use, and callers
/// only ever see definitions for their allowed set. Full MCP JSON-RPC
/// transport arrives with Phase 9 networking; the lazy/filtered contract
/// is established here so nothing above depends on eagerness.
pub struct Catalog {
    registry: Registry,
    factories: Mutex<HashMap<String, (ToolDef, ToolFactory)>>,
}

impl Catalog {
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            factories: Mutex::new(HashMap::new()),
        }
    }

    pub fn register_factory(
        &self,
        def: ToolDef,
        factory: impl Fn() -> Arc<dyn Tool> + Send + Sync + 'static,
    ) {
        self.factories
            .lock()
            .unwrap()
            .insert(def.name.clone(), (def, Box::new(factory)));
    }

    /// Instantiate on first use. Returns false when unknown.
    /// The factory is consumed on load (true lazy singleton); the
    /// definition stays visible via the registry afterwards.
    pub fn ensure_loaded(&self, name: &str) -> bool {
        if self.registry.has(name) {
            return true;
        }
        let factory = self.factories.lock().unwrap().remove(name);
        match factory {
            Some((_, f)) => {
                self.registry.register(f());
                true
            }
            None => false,
        }
    }

    pub fn execute(&self, name: &str, args: &str) -> Option<ToolResult> {
        if !self.ensure_loaded(name) {
            return None;
        }
        self.registry.execute(name, args)
    }

    pub async fn execute_async(&self, name: &str, args: &str) -> Option<ToolResult> {
        if !self.ensure_loaded(name) {
            return None;
        }
        self.registry.execute_async(name, args).await
    }

    /// Definitions for the allowed set, including not-yet-loaded tools.
    pub fn definitions_for(&self, allowed: &[String]) -> Vec<ToolDef> {
        let factories = self.factories.lock().unwrap();
        let mut out: Vec<ToolDef> = Vec::new();
        for name in allowed {
            if let Some((d, _)) = factories.get(name) {
                out.push(d.clone());
            }
        }
        drop(factories);
        // Merge defs of already-loaded tools (factories are consumed on load).
        for d in self.registry.definitions_for(allowed) {
            if !out.iter().any(|x| x.name == d.name) {
                out.push(d);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn loaded_names(&self) -> Vec<String> {
        self.registry.names()
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Native tools
// ---------------------------------------------------------------------------

fn parse_json_args(args: &str) -> Result<serde_json::Value, ToolError> {
    serde_json::from_str(args).map_err(|e| ToolError::BadArgs(format!("want JSON args: {e}")))
}

fn arg_str(v: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::BadArgs(format!("missing string field `{key}`")))
}

/// Filesystem confined to one root. Rejects absolute paths and `..`
/// lexically; the root is canonicalized at construction. (TOCTOU-hardened
/// confinement arrives with the sandbox layer in Phase 8.)
pub struct FilesystemTool {
    root: PathBuf,
}

impl FilesystemTool {
    pub fn new(root: PathBuf) -> Result<Self, ToolError> {
        let canonical = root
            .canonicalize()
            .map_err(|e| ToolError::Denied(format!("bad root: {e}")))?;
        Ok(Self { root: canonical })
    }

    fn confine(&self, rel: &str) -> Result<PathBuf, ToolError> {
        let p = std::path::Path::new(rel);
        if p.is_absolute() {
            return Err(ToolError::Denied("absolute paths rejected".into()));
        }
        if p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            return Err(ToolError::Denied("path traversal rejected".into()));
        }
        Ok(self.root.join(p))
    }
}

impl Tool for FilesystemTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "filesystem".into(),
            description:
                "Root-confined file read/list/write. JSON: {op: read|list|write, path, content?}"
                    .into(),
            version: "0.1.0".into(),
            permission: PermissionLevel::L1Write,
            risk: RiskLevel::Low,
            requires_network: false,
            timeout_ms: Some(10_000),
        }
    }

    fn validate(&self, args: &str) -> Result<(), ToolError> {
        let v = parse_json_args(args)?;
        let op = arg_str(&v, "op")?;
        if !["read", "list", "write"].contains(&op.as_str()) {
            return Err(ToolError::BadArgs(format!("unknown op `{op}`")));
        }
        let path = arg_str(&v, "path")?;
        self.confine(&path).map(|_| ())
    }

    fn run(&self, args: &str) -> ToolResult {
        let v = match parse_json_args(args) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(e.to_string()),
        };
        let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("");
        let rel = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let target = match self.confine(rel) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(e.to_string()),
        };
        match op {
            "read" => match std::fs::metadata(&target) {
                Ok(md) if md.is_dir() => ToolResult::err("path is a directory, use list".into()),
                Ok(md) if md.len() > MAX_FILE_READ_BYTES => {
                    ToolResult::err(format!("file too large ({} bytes)", md.len()))
                }
                Ok(_) => match std::fs::read_to_string(&target) {
                    Ok(s) => ToolResult::ok(s),
                    Err(e) => ToolResult::err(format!("read failed: {e}")),
                },
                Err(e) => ToolResult::err(format!("read failed: {e}")),
            },
            "list" => match std::fs::read_dir(&target) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect();
                    names.sort();
                    ToolResult::ok(names.join("\n"))
                }
                Err(e) => ToolResult::err(format!("list failed: {e}")),
            },
            "write" => {
                let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(parent) = target.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return ToolResult::err(format!("mkdir failed: {e}"));
                    }
                }
                match std::fs::write(&target, content) {
                    Ok(()) => ToolResult::ok(format!("wrote {} bytes", content.len())),
                    Err(e) => ToolResult::err(format!("write failed: {e}")),
                }
            }
            _ => ToolResult::err(format!("unknown op `{op}`")),
        }
    }
}

/// Shell execution with a denylist and registry-level timeouts.
/// JSON: {cmd}. Never runs denied patterns, even if the model asks nicely.
pub struct TerminalTool {
    pub work_dir: Option<PathBuf>,
    pub extra_denied: Vec<String>,
}

const DENIED_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "mkfs",
    "dd if=",
    ":(){",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "format c:",
    "format d:",
    "del /f /s /q",
    "rd /s /q c:",
    "takeown",
    "> /dev/sda",
    "chmod -r 777 /",
    "curl|sh",
    "wget|sh",
];

impl TerminalTool {
    pub fn new() -> Self {
        Self {
            work_dir: None,
            extra_denied: vec![],
        }
    }

    fn denied(&self, cmd: &str) -> bool {
        let lower = cmd.to_lowercase();
        DENIED_PATTERNS.iter().any(|p| lower.contains(p))
            || self
                .extra_denied
                .iter()
                .any(|p| lower.contains(&p.to_lowercase()))
    }
}

impl Default for TerminalTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for TerminalTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "terminal".into(),
            description: "Shell command execution with denylist + timeout. JSON: {cmd}".into(),
            version: "0.1.0".into(),
            permission: PermissionLevel::L2Exec,
            risk: RiskLevel::High,
            requires_network: false,
            timeout_ms: Some(30_000),
        }
    }

    fn validate(&self, args: &str) -> Result<(), ToolError> {
        let v = parse_json_args(args)?;
        let cmd = arg_str(&v, "cmd")?;
        if cmd.trim().is_empty() {
            return Err(ToolError::BadArgs("empty cmd".into()));
        }
        if self.denied(&cmd) {
            return Err(ToolError::Denied(format!("blocked command pattern: {cmd}")));
        }
        Ok(())
    }

    fn run(&self, args: &str) -> ToolResult {
        let v = match parse_json_args(args) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(e.to_string()),
        };
        let cmd = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
        if self.denied(cmd) {
            return ToolResult::err(
                ToolError::Denied("blocked command pattern".into()).to_string(),
            );
        }
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.arg("/C").arg(cmd);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(cmd);
            c
        };
        if let Some(dir) = &self.work_dir {
            command.current_dir(dir);
        }
        match command.output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if out.status.success() {
                    ToolResult::ok(stdout)
                } else {
                    ToolResult::err(format!("exit {}: {}", out.status, stderr.trim()))
                }
            }
            Err(e) => ToolResult::err(ToolError::Exec(e.to_string()).to_string()),
        }
    }
}

/// Web fetch placeholder. Validates http(s) URLs today; the actual
/// transport arrives with Phase 9 networking (reqwest/rustls), so this
/// honestly refuses instead of faking a fetch.
pub struct BrowserTool;

impl Tool for BrowserTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "browser".into(),
            description: "Web fetch (URL validation now; transport in Phase 9). JSON: {url}".into(),
            version: "0.1.0".into(),
            permission: PermissionLevel::L1Write,
            risk: RiskLevel::Low,
            requires_network: true,
            timeout_ms: Some(15_000),
        }
    }

    fn validate(&self, args: &str) -> Result<(), ToolError> {
        let v = parse_json_args(args)?;
        let url = arg_str(&v, "url")?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ToolError::BadArgs("only http(s) URLs allowed".into()));
        }
        if url.len() > 2048 {
            return Err(ToolError::BadArgs("url too long".into()));
        }
        Ok(())
    }

    fn run(&self, _args: &str) -> ToolResult {
        ToolResult::err("browser fetch transport not wired (Phase 9 networking)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Echo;
    impl Tool for Echo {
        fn definition(&self) -> ToolDef {
            ToolDef {
                name: "echo".into(),
                description: "echo".into(),
                version: "0.1.0".into(),
                permission: PermissionLevel::L0Read,
                risk: RiskLevel::None,
                requires_network: false,
                timeout_ms: None,
            }
        }
        fn run(&self, args: &str) -> ToolResult {
            ToolResult::ok(args.into())
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("igris-tools-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn registry_roundtrip() {
        let r = Registry::new();
        r.register(Arc::new(Echo));
        let out = r.execute("echo", "hi").unwrap();
        assert!(out.ok && out.output == "hi");
        assert_eq!(r.success_rate("echo"), Some(1.0));
    }

    #[test]
    fn risk_gating() {
        assert!(!RiskLevel::Low.needs_approval());
        assert!(!RiskLevel::None.needs_approval());
        assert!(RiskLevel::Medium.needs_approval());
        assert!(RiskLevel::High.needs_approval());
        assert!(RiskLevel::Critical.needs_approval());
        assert_eq!(TerminalTool::new().definition().risk, RiskLevel::High);
    }

    #[test]
    fn filesystem_roundtrip() {
        let root = temp_root("fs");
        let fs = FilesystemTool::new(root).unwrap();
        let r = Registry::new();
        r.register(Arc::new(fs));
        let w = r
            .execute(
                "filesystem",
                r#"{"op":"write","path":"notes/a.txt","content":"hello jarvis"}"#,
            )
            .unwrap();
        assert!(w.ok, "{w:?}");
        let rd = r
            .execute("filesystem", r#"{"op":"read","path":"notes/a.txt"}"#)
            .unwrap();
        assert!(rd.ok && rd.output == "hello jarvis");
        let ls = r
            .execute("filesystem", r#"{"op":"list","path":"notes"}"#)
            .unwrap();
        assert!(ls.ok && ls.output.contains("a.txt"));
    }

    #[test]
    fn filesystem_traversal_blocked() {
        let root = temp_root("trav");
        let fs = FilesystemTool::new(root).unwrap();
        let r = Registry::new();
        r.register(Arc::new(fs));
        for bad in [
            r#"{"op":"read","path":"../evil.txt"}"#,
            r#"{"op":"read","path":"/abs.txt"}"#,
            r#"{"op":"zap","path":"x"}"#,
        ] {
            let out = r.execute("filesystem", bad).unwrap();
            assert!(!out.ok, "{bad} must fail");
        }
    }

    #[test]
    fn terminal_echo_and_denied() {
        let r = Registry::new();
        r.register(Arc::new(TerminalTool::new()));
        let ok = r.execute("terminal", r#"{"cmd":"echo hello"}"#).unwrap();
        assert!(ok.ok && ok.output.contains("hello"), "{ok:?}");
        let denied = r
            .execute("terminal", r#"{"cmd":"rm -rf / --no-preserve-root"}"#)
            .unwrap();
        assert!(!denied.ok && denied.error.unwrap().contains("blocked"));
        let empty = r.execute("terminal", r#"{"cmd":"  "}"#).unwrap();
        assert!(!empty.ok);
    }

    struct Sleepy;
    impl Tool for Sleepy {
        fn definition(&self) -> ToolDef {
            ToolDef {
                name: "sleepy".into(),
                description: "blocks".into(),
                version: "0.1.0".into(),
                permission: PermissionLevel::L0Read,
                risk: RiskLevel::None,
                requires_network: false,
                timeout_ms: None,
            }
        }
        fn run(&self, _args: &str) -> ToolResult {
            std::thread::sleep(Duration::from_millis(500));
            ToolResult::ok("woke".into())
        }
    }

    #[tokio::test]
    async fn async_timeout_caps_blocking_tool() {
        let r = Registry::new();
        r.register(Arc::new(Sleepy));
        let timed = r
            .execute_async_with_timeout("sleepy", "{}", 50)
            .await
            .unwrap();
        assert!(!timed.ok && timed.error.unwrap().contains("timed out"));
        // Failure is audited.
        assert_eq!(r.success_rate("sleepy"), Some(0.0));
    }

    #[tokio::test]
    async fn async_success_path() {
        let r = Registry::new();
        r.register(Arc::new(Echo));
        let out = r.execute_async("echo", "yo").await.unwrap();
        assert!(out.ok && out.output == "yo");
    }

    #[test]
    fn browser_validates_but_defers_transport() {
        let r = Registry::new();
        r.register(Arc::new(BrowserTool));
        let bad = r.execute("browser", r#"{"url":"ftp://x/y"}"#).unwrap();
        assert!(!bad.ok);
        let ok_url = r
            .execute("browser", r#"{"url":"https://example.com"}"#)
            .unwrap();
        assert!(!ok_url.ok && ok_url.error.unwrap().contains("Phase 9"));
    }

    #[test]
    fn definitions_for_filters() {
        let r = Registry::new();
        r.register(Arc::new(Echo));
        r.register(Arc::new(BrowserTool));
        let defs = r.definitions_for(&["echo".to_string()]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
        assert!(r.definitions_for(&[]).is_empty());
    }

    #[test]
    fn catalog_lazy_loads_once() {
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl Tool for Counted {
            fn definition(&self) -> ToolDef {
                ToolDef {
                    name: "counted".into(),
                    description: "c".into(),
                    version: "0.1.0".into(),
                    permission: PermissionLevel::L0Read,
                    risk: RiskLevel::None,
                    requires_network: false,
                    timeout_ms: None,
                }
            }
            fn run(&self, args: &str) -> ToolResult {
                ToolResult::ok(args.into())
            }
        }
        let c = Catalog::new();
        c.register_factory(Counted.definition(), || {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Arc::new(Counted)
        });
        // Visible before loading (lazy exposure).
        let defs = c.definitions_for(&["counted".to_string()]);
        assert_eq!(defs.len(), 1);
        assert!(c.loaded_names().is_empty());
        // First execute instantiates exactly once.
        assert!(c.execute("counted", "x").unwrap().ok);
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
        assert_eq!(c.loaded_names(), vec!["counted".to_string()]);
        assert!(c.execute("ghost", "x").is_none());
    }
}
