//! igris-agents — typed capabilities, least-privilege workers.
//!
//! Phase 6: six role shells with tool scopes, deterministic task planning
//! (LLM-backed execution lands with the reasoning pipeline), handoffs,
//! and A2A task/artifact contracts with untrusted-external validation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Roles + definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    Planner,
    Coder,
    Researcher,
    Developer,
    DevOps,
    Communicator,
    /// User-facing front door: talks to the user, clarifies intent, and hands
    /// work to the orchestrator, which routes it to the other agents.
    Assistant,
}

impl AgentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Planner => "planner",
            AgentRole::Coder => "coder",
            AgentRole::Researcher => "researcher",
            AgentRole::Developer => "developer",
            AgentRole::DevOps => "devops",
            AgentRole::Communicator => "communicator",
            AgentRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub role: AgentRole,
    pub capabilities: Vec<String>,
    pub allowed_tools: Vec<String>,
    /// 0 = chat, 1 = reasoning, 2 = worker. Workers cannot delegate further.
    pub tier: u8,
    pub max_iterations: u32,
    pub system_prompt: String,
}

impl AgentDef {
    fn base(
        role: AgentRole,
        capabilities: &[&str],
        allowed_tools: &[&str],
        tier: u8,
        max_iterations: u32,
        system_prompt: &str,
    ) -> Self {
        Self {
            name: role.as_str().into(),
            role,
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            allowed_tools: allowed_tools.iter().map(|s| s.to_string()).collect(),
            tier,
            max_iterations,
            system_prompt: system_prompt.into(),
        }
    }

    pub fn planner() -> Self {
        Self::base(
            AgentRole::Planner,
            &["decompose_goal", "order_tasks", "estimate_risk"],
            &["memory_search"],
            1,
            5,
            "Decompose goals into ordered, verifiable steps. Never execute tools directly.",
        )
    }

    pub fn coder() -> Self {
        Self::base(
            AgentRole::Coder,
            &["read_repository", "modify_source", "run_tests"],
            &["filesystem", "terminal"],
            2,
            20,
            "Read before writing. Smallest diff that satisfies the task. Run tests.",
        )
    }

    pub fn researcher() -> Self {
        Self::base(
            AgentRole::Researcher,
            &["web_read", "compare", "summarize"],
            &["browser"],
            2,
            10,
            "Cite sources. Compare alternatives. No side effects.",
        )
    }

    pub fn developer() -> Self {
        Self::base(
            AgentRole::Developer,
            &["scaffold_project", "install_deps", "build", "package"],
            &["filesystem", "terminal"],
            2,
            15,
            "Reproducible builds. Pin versions. Verify artifacts.",
        )
    }

    pub fn devops() -> Self {
        Self::base(
            AgentRole::DevOps,
            &["build_image", "deploy", "monitor", "rollback"],
            &["terminal", "docker"],
            2,
            15,
            "Never deploy without a rollback plan. Confirm destructive ops.",
        )
    }

    pub fn communicator() -> Self {
        Self::base(
            AgentRole::Communicator,
            &["draft_message", "summarize_thread", "schedule_reminder"],
            &["messaging", "memory_search"],
            2,
            5,
            "Drafts only. External sends always require user approval.",
        )
    }

    pub fn assistant() -> Self {
        Self::base(
            AgentRole::Assistant,
            &[
                "greet_user",
                "clarify_intent",
                "delegate_task",
                "present_results",
            ],
            &["memory_search", "messaging"],
            1,
            10,
            "You are the user's front door. Clarify vague requests, load user context, \
             then hand work to the orchestrator with a crisp intent — never execute \
             worker tools yourself. Present results back conversationally.",
        )
    }

    pub fn all_roles() -> Vec<Self> {
        vec![
            Self::planner(),
            Self::coder(),
            Self::researcher(),
            Self::developer(),
            Self::devops(),
            Self::communicator(),
            Self::assistant(),
        ]
    }

    pub fn can_use(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|t| t == tool)
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Workers (tier 2) cannot delegate to other agents.
    pub fn can_delegate(&self) -> bool {
        self.tier < 2
    }
}

// ---------------------------------------------------------------------------
// Tasks + deterministic planning stub
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error("tool not in scope: {0}")]
    ToolNotInScope(String),
    #[error("unknown role for task: {0}")]
    UnknownRole(String),
    #[error("empty task description")]
    EmptyTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: Uuid,
    pub description: String,
    pub input: String,
    pub requested_tools: Vec<String>,
}

impl AgentTask {
    pub fn new(description: &str, input: &str, requested_tools: &[&str]) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            input: input.into(),
            requested_tools: requested_tools.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub tool: String,
    pub args_hint: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub steps: Vec<PlanStep>,
}

/// Minimal execution context: the tools this run may consider.
/// Real memory + provider wiring arrives with the reasoning pipeline;
/// this stub plans deterministically so missions can already orchestrate.
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    pub available_tools: Vec<String>,
}

/// Validate scope, then emit a role-specific step plan.
/// No LLM is invoked; `output` is a human-readable plan summary.
pub fn execute_task(
    def: &AgentDef,
    task: &AgentTask,
    _ctx: &AgentContext,
) -> Result<AgentResult, AgentError> {
    if task.description.trim().is_empty() {
        return Err(AgentError::EmptyTask);
    }
    for t in &task.requested_tools {
        if !def.can_use(t) {
            return Err(AgentError::ToolNotInScope(t.clone()));
        }
    }
    let steps = plan_for(def.role, task);
    let output = format!(
        "{} plan ({} steps): {}",
        def.name,
        steps.len(),
        steps
            .iter()
            .map(|s| s.description.clone())
            .collect::<Vec<_>>()
            .join(" → ")
    );
    Ok(AgentResult {
        success: true,
        output,
        error: None,
        steps,
    })
}

fn step(tool: &str, args_hint: &str, description: &str) -> PlanStep {
    PlanStep {
        tool: tool.into(),
        args_hint: args_hint.into(),
        description: description.into(),
    }
}

fn plan_for(role: AgentRole, task: &AgentTask) -> Vec<PlanStep> {
    let input = task.input.clone();
    match role {
        AgentRole::Planner => vec![
            step("memory_search", &input, "gather relevant context"),
            step("memory_search", &input, "decompose goal into ordered steps"),
            step("memory_search", &input, "attach verification criteria"),
        ],
        AgentRole::Coder => vec![
            step("filesystem", &input, "read relevant sources"),
            step("filesystem", &input, "apply minimal diff"),
            step("terminal", "cargo test", "run tests"),
        ],
        AgentRole::Researcher => vec![
            step("browser", &input, "read primary sources"),
            step("browser", &input, "compare alternatives and summarize"),
        ],
        AgentRole::Developer => vec![
            step("filesystem", &input, "scaffold project layout"),
            step("terminal", "install + build", "install deps and build"),
            step("terminal", "verify artifact", "verify packaged artifact"),
        ],
        AgentRole::DevOps => vec![
            step("terminal", "build image", "build container image"),
            step("docker", &input, "deploy with rollback plan"),
            step("terminal", "health check", "verify health post-deploy"),
        ],
        AgentRole::Communicator => vec![
            step("memory_search", &input, "load tone and thread context"),
            step(
                "messaging",
                &input,
                "draft message (approval required to send)",
            ),
        ],
        AgentRole::Assistant => vec![
            step("memory_search", &input, "load user context and preferences"),
            step("memory_search", &input, "clarify intent into a crisp task"),
            step(
                "messaging",
                &input,
                "hand off to orchestrator for routing to workers",
            ),
            step("messaging", &input, "present results back to the user"),
        ],
    }
}

// ---------------------------------------------------------------------------
// Handoffs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub from: AgentRole,
    pub to: AgentRole,
    pub reason: String,
    pub context_summary: String,
}

/// Handoffs require a real reason, a different target, and a delegating source.
pub fn can_handoff(from: &AgentDef, to_role: AgentRole, reason: &str) -> bool {
    from.can_delegate() && from.role != to_role && !reason.trim().is_empty()
}

// ---------------------------------------------------------------------------
// A2A contracts (untrusted externals validated, never trusted)
// ---------------------------------------------------------------------------

pub mod a2a {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AgentCard {
        pub name: String,
        pub version: String,
        pub capabilities: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AgentTaskRequest {
        pub id: Uuid,
        pub intent: String,
        pub capability: String,
        pub input: String,
        pub requester: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Artifact {
        pub id: Uuid,
        pub task_id: Uuid,
        pub producer: String,
        pub content: String,
        pub content_hash: u64,
    }

    #[derive(Debug, Clone, thiserror::Error)]
    pub enum A2aError {
        #[error("capability not advertised: {0}")]
        CapabilityNotAdvertised(String),
        #[error("empty input")]
        EmptyInput,
        #[error("artifact hash mismatch for: {0}")]
        HashMismatch(String),
        #[error("unknown producer: {0}")]
        UnknownProducer(String),
    }

    pub fn hash_content(content: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        content.hash(&mut h);
        h.finish()
    }

    /// Validate an incoming task against the producer's own card.
    pub fn validate_task(card: &AgentCard, req: &AgentTaskRequest) -> Result<(), A2aError> {
        if !card.capabilities.iter().any(|c| c == &req.capability) {
            return Err(A2aError::CapabilityNotAdvertised(req.capability.clone()));
        }
        if req.input.trim().is_empty() {
            return Err(A2aError::EmptyInput);
        }
        Ok(())
    }

    pub fn make_artifact(task_id: Uuid, producer: &str, content: &str) -> Artifact {
        Artifact {
            id: Uuid::new_v4(),
            task_id,
            producer: producer.into(),
            content: content.into(),
            content_hash: hash_content(content),
        }
    }

    /// Verify an artifact: known producer, intact content.
    pub fn verify_artifact(known_producers: &[&str], art: &Artifact) -> Result<(), A2aError> {
        if !known_producers.contains(&art.producer.as_str()) {
            return Err(A2aError::UnknownProducer(art.producer.clone()));
        }
        if hash_content(&art.content) != art.content_hash {
            return Err(A2aError::HashMismatch(art.id.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a2a::*;

    fn ctx() -> AgentContext {
        AgentContext {
            available_tools: vec!["filesystem".into(), "terminal".into()],
        }
    }

    #[test]
    fn least_privilege() {
        let c = AgentDef::coder();
        assert!(c.can_use("terminal"));
        assert!(!c.can_use("gmail.send"));
    }

    #[test]
    fn seven_roles_defined() {
        let roles = AgentDef::all_roles();
        assert_eq!(roles.len(), 7);
        assert!(roles
            .iter()
            .all(|r| !r.capabilities.is_empty() && !r.system_prompt.is_empty()));
        // No role gets send-capable tools: external sends stay approval-gated.
        for r in &roles {
            assert!(
                !r.can_use("gmail.send"),
                "{} must not send directly",
                r.name
            );
        }
    }

    #[test]
    fn assistant_is_delegating_front_door() {
        let a = AgentDef::assistant();
        assert_eq!(a.role, AgentRole::Assistant);
        assert!(
            a.can_delegate(),
            "assistant must delegate to the orchestrator"
        );
        assert!(a.supports("delegate_task"));
        // Front door never touches worker tools directly.
        assert!(!a.can_use("terminal"));
        assert!(!a.can_use("filesystem"));
        assert!(!a.can_use("docker"));
    }

    #[test]
    fn assistant_plan_routes_via_orchestrator() {
        let def = AgentDef::assistant();
        let task = AgentTask::new(
            "help me",
            "fix the router bug",
            &["memory_search", "messaging"],
        );
        let res = execute_task(&def, &task, &ctx()).unwrap();
        assert!(res.success);
        assert!(res
            .steps
            .iter()
            .any(|s| s.description.contains("orchestrator")));
        for s in &res.steps {
            assert!(
                def.can_use(&s.tool),
                "assistant plan step uses out-of-scope tool {}",
                s.tool
            );
        }
    }

    #[test]
    fn assistant_hands_off_to_workers() {
        let a = AgentDef::assistant();
        assert!(can_handoff(&a, AgentRole::Coder, "needs implementation"));
        assert!(can_handoff(&a, AgentRole::Planner, "needs decomposition"));
        assert!(!can_handoff(&a, AgentRole::Assistant, "same role"));
    }

    #[test]
    fn workers_cannot_delegate() {
        assert!(!AgentDef::coder().can_delegate());
        assert!(AgentDef::planner().can_delegate());
    }

    #[test]
    fn execute_rejects_out_of_scope_tool() {
        let def = AgentDef::researcher();
        let task = AgentTask::new("research x", "x", &["terminal"]);
        let err = execute_task(&def, &task, &ctx()).unwrap_err();
        assert!(matches!(err, AgentError::ToolNotInScope(_)));
    }

    #[test]
    fn execute_rejects_empty_task() {
        let def = AgentDef::coder();
        let task = AgentTask::new("  ", "x", &[]);
        assert!(matches!(
            execute_task(&def, &task, &ctx()),
            Err(AgentError::EmptyTask)
        ));
    }

    #[test]
    fn coder_plan_uses_only_scoped_tools() {
        let def = AgentDef::coder();
        let task = AgentTask::new("fix router", "router bug", &["filesystem", "terminal"]);
        let res = execute_task(&def, &task, &ctx()).unwrap();
        assert!(res.success && res.steps.len() >= 3);
        for s in &res.steps {
            assert!(
                def.can_use(&s.tool),
                "plan step uses out-of-scope tool {}",
                s.tool
            );
        }
    }

    #[test]
    fn handoff_rules() {
        let planner = AgentDef::planner();
        assert!(can_handoff(
            &planner,
            AgentRole::Coder,
            "needs implementation"
        ));
        assert!(!can_handoff(&planner, AgentRole::Planner, "same role"));
        assert!(!can_handoff(&planner, AgentRole::Coder, "  "));
        assert!(!can_handoff(
            &AgentDef::coder(),
            AgentRole::Planner,
            "worker cannot delegate"
        ));
    }

    #[test]
    fn a2a_accepts_advertised_capability() {
        let card = AgentCard {
            name: "coder".into(),
            version: "0.1".into(),
            capabilities: vec!["run_tests".into()],
        };
        let req = AgentTaskRequest {
            id: Uuid::new_v4(),
            intent: "verify".into(),
            capability: "run_tests".into(),
            input: "cargo test".into(),
            requester: "planner".into(),
        };
        assert!(validate_task(&card, &req).is_ok());
    }

    #[test]
    fn a2a_rejects_unadvertised_and_empty() {
        let card = AgentCard {
            name: "x".into(),
            version: "0.1".into(),
            capabilities: vec!["a".into()],
        };
        let bad_cap = AgentTaskRequest {
            id: Uuid::new_v4(),
            intent: "i".into(),
            capability: "root_shell".into(),
            input: "x".into(),
            requester: "e".into(),
        };
        assert!(matches!(
            validate_task(&card, &bad_cap),
            Err(A2aError::CapabilityNotAdvertised(_))
        ));
        let empty = AgentTaskRequest {
            capability: "a".into(),
            input: "  ".into(),
            ..bad_cap
        };
        assert!(matches!(
            validate_task(&card, &empty),
            Err(A2aError::EmptyInput)
        ));
    }

    #[test]
    fn artifact_roundtrip_and_tamper() {
        let task_id = Uuid::new_v4();
        let art = make_artifact(task_id, "coder", "all tests pass");
        assert!(verify_artifact(&["coder"], &art).is_ok());
        assert!(matches!(
            verify_artifact(&["stranger"], &art),
            Err(A2aError::UnknownProducer(_))
        ));
        let mut tampered = art.clone();
        tampered.content = "all tests pass (edited)".into();
        assert!(matches!(
            verify_artifact(&["coder"], &tampered),
            Err(A2aError::HashMismatch(_))
        ));
    }
}
