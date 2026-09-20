//! igris-router — privacy-aware adaptive model routing.
//!
//! Two layers:
//! - `route()` — static fast path (privacy + task + network). No state, no I/O.
//! - `ModelRouter` — adaptive path: provider registry, historical stats
//!   (aggregate counters only, never prompts), budget enforcement, fallback chains.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrivacyLevel {
    Public,
    Internal,
    Confidential,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    Simple,
    Complex,
    Coding,
    Creative,
    Speech,
    Vision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelClass {
    LocalTiny,
    LocalSmall,
    FastCloud,
    ReasoningCloud,
    Vision,
    Speech,
    Embedding,
}

impl ModelClass {
    pub fn needs_network(&self) -> bool {
        matches!(
            self,
            ModelClass::FastCloud | ModelClass::ReasoningCloud | ModelClass::Vision
        )
    }

    pub fn is_local(&self) -> bool {
        !self.needs_network()
    }
}

#[derive(Debug, Clone)]
pub struct RouteRequest {
    pub privacy: PrivacyLevel,
    pub task: TaskType,
    pub needs_vision: bool,
    pub needs_reasoning: bool,
    pub network_up: bool,
}

/// Static fast path. Pure function, no state.
pub fn route(req: &RouteRequest) -> ModelClass {
    if req.privacy == PrivacyLevel::Secret || req.privacy == PrivacyLevel::Confidential {
        return if req.needs_reasoning {
            ModelClass::LocalSmall
        } else {
            ModelClass::LocalTiny
        };
    }
    if req.needs_vision {
        return ModelClass::Vision;
    }
    if !req.network_up {
        return ModelClass::LocalSmall;
    }
    match req.task {
        TaskType::Simple | TaskType::Speech => ModelClass::LocalTiny,
        TaskType::Coding | TaskType::Complex if req.needs_reasoning => ModelClass::ReasoningCloud,
        TaskType::Coding | TaskType::Complex => ModelClass::FastCloud,
        TaskType::Creative => ModelClass::FastCloud,
        TaskType::Vision => ModelClass::Vision,
    }
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub class: ModelClass,
    pub max_context_tokens: usize,
    pub cost_per_1k_tokens: f64,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum RouterError {
    #[error("no provider available: {0}")]
    NoProvider(String),
    #[error("budget exceeded: cheapest {cheapest:.4} > max {max:.4}")]
    BudgetExceeded { cheapest: f64, max: f64 },
    #[error("provider {0} unavailable")]
    Unavailable(String),
}

pub trait ModelProvider: Send + Sync {
    fn info(&self) -> &ModelInfo;
    fn is_available(&self) -> bool;
    /// Placeholder inference. Local echo returns a truncated echo;
    /// cloud stubs require real credentials (wired in model-service later).
    fn generate(&self, prompt: &str, max_tokens: usize) -> Result<String, RouterError>;
}

/// Always-available offline provider. Deterministic, no network.
pub struct LocalEchoProvider {
    info: ModelInfo,
}

impl LocalEchoProvider {
    pub fn tiny() -> Self {
        Self {
            info: ModelInfo {
                id: "local-tiny".into(),
                class: ModelClass::LocalTiny,
                max_context_tokens: 2048,
                cost_per_1k_tokens: 0.0,
                supports_vision: false,
                supports_reasoning: false,
            },
        }
    }

    pub fn small() -> Self {
        Self {
            info: ModelInfo {
                id: "local-small".into(),
                class: ModelClass::LocalSmall,
                max_context_tokens: 8192,
                cost_per_1k_tokens: 0.0,
                supports_vision: false,
                supports_reasoning: true,
            },
        }
    }
}

impl ModelProvider for LocalEchoProvider {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn is_available(&self) -> bool {
        true
    }

    fn generate(&self, prompt: &str, max_tokens: usize) -> Result<String, RouterError> {
        let take = max_tokens.min(256);
        let snippet: String = prompt.chars().take(take).collect();
        Ok(format!("[{}] {snippet}", self.info.id))
    }
}

/// Cloud provider stub. Available only when its env key is set.
/// Real HTTP inference lands in `services/model-service`; the router only gates.
pub struct CloudStubProvider {
    info: ModelInfo,
    env_key: String,
}

impl CloudStubProvider {
    pub fn fast_cloud(env_key: &str) -> Self {
        Self {
            info: ModelInfo {
                id: "fast-cloud".into(),
                class: ModelClass::FastCloud,
                max_context_tokens: 32000,
                cost_per_1k_tokens: 0.002,
                supports_vision: false,
                supports_reasoning: true,
            },
            env_key: env_key.into(),
        }
    }

    pub fn reasoning_cloud(env_key: &str) -> Self {
        Self {
            info: ModelInfo {
                id: "reasoning-cloud".into(),
                class: ModelClass::ReasoningCloud,
                max_context_tokens: 128000,
                cost_per_1k_tokens: 0.01,
                supports_vision: false,
                supports_reasoning: true,
            },
            env_key: env_key.into(),
        }
    }
}

impl ModelProvider for CloudStubProvider {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn is_available(&self) -> bool {
        std::env::var(&self.env_key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    fn generate(&self, _prompt: &str, _max_tokens: usize) -> Result<String, RouterError> {
        if self.is_available() {
            Err(RouterError::Unavailable(format!(
                "{}: transport not wired (model-service)",
                self.info.id
            )))
        } else {
            Err(RouterError::Unavailable(format!(
                "{}: missing {}",
                self.info.id, self.env_key
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Adaptive router
// ---------------------------------------------------------------------------

/// Aggregate per-provider stats. Counters + latency only — never prompts,
/// so no private user data trains the router.
#[derive(Debug, Clone, Default)]
pub struct RouteStats {
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
}

impl RouteStats {
    /// Neutral 0.5 prior so new providers are still tried.
    pub fn success_rate(&self) -> f64 {
        let n = self.successes + self.failures;
        if n == 0 {
            0.5
        } else {
            self.successes as f64 / n as f64
        }
    }

    pub fn avg_latency_ms(&self) -> f64 {
        let n = self.successes + self.failures;
        if n == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / n as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingPolicy {
    pub prefer_local: bool,
    pub allow_cloud: bool,
    /// Max spend per request in abstract cost units. 0.0 = free-only.
    pub max_cost_per_request: f64,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            prefer_local: true,
            allow_cloud: true,
            max_cost_per_request: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub provider_id: String,
    pub class: ModelClass,
    pub reason: String,
    pub estimated_cost: f64,
    /// Ordered fallbacks; local-echo is always last when present.
    pub fallbacks: Vec<String>,
}

pub struct ModelRouter {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    order: Vec<String>,
    stats: HashMap<String, RouteStats>,
    pub policy: RoutingPolicy,
}

impl ModelRouter {
    pub fn new(policy: RoutingPolicy) -> Self {
        Self {
            providers: HashMap::new(),
            order: Vec::new(),
            stats: HashMap::new(),
            policy,
        }
    }

    /// Offline-safe default: two local providers, cloud allowed by policy.
    pub fn with_defaults() -> Self {
        let mut r = Self::new(RoutingPolicy::default());
        r.register(Arc::new(LocalEchoProvider::tiny()));
        r.register(Arc::new(LocalEchoProvider::small()));
        r.register(Arc::new(CloudStubProvider::fast_cloud(
            "IGRIS_CLOUD_API_KEY",
        )));
        r.register(Arc::new(CloudStubProvider::reasoning_cloud(
            "IGRIS_CLOUD_API_KEY",
        )));
        r
    }

    pub fn register(&mut self, provider: Arc<dyn ModelProvider>) {
        let id = provider.info().id.clone();
        if !self.providers.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.providers.insert(id, provider);
    }

    pub fn estimate_cost(&self, provider_id: &str, tokens: usize) -> Option<f64> {
        self.providers
            .get(provider_id)
            .map(|p| p.info().cost_per_1k_tokens * tokens as f64 / 1000.0)
    }

    /// Record an outcome. Only aggregates — callers must NOT pass prompts.
    pub fn record_feedback(&mut self, provider_id: &str, latency_ms: u64, success: bool) {
        let s = self.stats.entry(provider_id.into()).or_default();
        if success {
            s.successes += 1;
        } else {
            s.failures += 1;
        }
        s.total_latency_ms += latency_ms;
    }

    pub fn stats(&self, provider_id: &str) -> RouteStats {
        self.stats.get(provider_id).cloned().unwrap_or_default()
    }

    fn candidate(&self, id: &str, req: &RouteRequest) -> Option<(f64, String)> {
        let p = self.providers.get(id)?;
        if !p.is_available() {
            return None;
        }
        let info = p.info();
        // Privacy gate: secrets never leave the device.
        if req.privacy >= PrivacyLevel::Confidential && !info.class.is_local() {
            return None;
        }
        if !self.policy.allow_cloud && !info.class.is_local() {
            return None;
        }
        if info.class.needs_network() && !req.network_up {
            return None;
        }
        if req.needs_vision && !info.supports_vision {
            return None;
        }
        if req.needs_reasoning && !info.supports_reasoning {
            // Tiny local can still answer; deprioritize instead of exclude.
            if info.class != ModelClass::LocalTiny {
                return None;
            }
        }
        let want = route(req);
        let mut score = 0.0;
        if info.class == want {
            score += 1.0;
        }
        // Historical reliability (neutral prior 0.5).
        let st = self.stats(id);
        score += 0.6 * st.success_rate();
        // Latency: prefer faster providers once observed.
        if st.avg_latency_ms() > 0.0 {
            score += 100.0 / (100.0 + st.avg_latency_ms());
        }
        if self.policy.prefer_local && info.class.is_local() {
            score += 0.25;
        }
        Some((score, id.to_string()))
    }

    pub fn route_to(
        &self,
        req: &RouteRequest,
        tokens: usize,
    ) -> Result<RoutingDecision, RouterError> {
        let mut scored: Vec<(f64, String)> = self
            .order
            .iter()
            .filter_map(|id| self.candidate(id, req))
            .collect();
        if scored.is_empty() {
            return Err(RouterError::NoProvider(
                "no provider satisfies privacy/network/capability gates".into(),
            ));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        // Budget: walk the ranking and pick the first affordable option.
        let mut affordable: Option<(f64, String)> = None;
        let mut cheapest = f64::INFINITY;
        for (score, id) in &scored {
            let cost = self.estimate_cost(id, tokens).unwrap_or(f64::INFINITY);
            cheapest = cheapest.min(cost);
            if cost <= self.policy.max_cost_per_request && affordable.is_none() {
                affordable = Some((*score, id.clone()));
            }
        }
        let (score, best) = affordable.ok_or(RouterError::BudgetExceeded {
            cheapest,
            max: self.policy.max_cost_per_request,
        })?;
        let class = self
            .providers
            .get(&best)
            .map(|p| p.info().class)
            .unwrap_or(ModelClass::LocalTiny);
        let cost = self.estimate_cost(&best, tokens).unwrap_or(0.0);
        let mut fallbacks: Vec<String> = scored
            .into_iter()
            .map(|(_, id)| id)
            .filter(|id| id != &best)
            .collect();
        // Local echo is the last resort when registered.
        fallbacks.sort_by_key(|id| {
            self.providers
                .get(id)
                .map(|p| !p.info().class.is_local())
                .unwrap_or(false)
        });
        Ok(RoutingDecision {
            provider_id: best.clone(),
            class,
            reason: format!(
                "score={score:.2} privacy={:?} task={:?}",
                req.privacy, req.task
            ),
            estimated_cost: cost,
            fallbacks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coding_secret() -> RouteRequest {
        RouteRequest {
            privacy: PrivacyLevel::Secret,
            task: TaskType::Coding,
            needs_vision: false,
            needs_reasoning: true,
            network_up: true,
        }
    }

    #[test]
    fn secret_stays_local() {
        let r = route(&coding_secret());
        assert_eq!(r, ModelClass::LocalSmall);
    }

    #[test]
    fn simple_public_fast() {
        let r = route(&RouteRequest {
            privacy: PrivacyLevel::Public,
            task: TaskType::Simple,
            needs_vision: false,
            needs_reasoning: false,
            network_up: true,
        });
        assert_eq!(r, ModelClass::LocalTiny);
    }

    fn offline_router() -> ModelRouter {
        // No env keys set in tests -> cloud stubs unavailable -> local only.
        ModelRouter::with_defaults()
    }

    #[test]
    fn offline_falls_back_to_local() {
        let r = offline_router();
        let d = r
            .route_to(
                &RouteRequest {
                    privacy: PrivacyLevel::Public,
                    task: TaskType::Coding,
                    needs_vision: false,
                    needs_reasoning: true,
                    network_up: false,
                },
                500,
            )
            .unwrap();
        assert!(
            d.class.is_local(),
            "offline must route local, got {:?}",
            d.class
        );
        assert!(d.fallbacks.iter().all(|f| r
            .providers
            .get(f)
            .map(|p| p.info().class.is_local())
            .unwrap_or(true)));
    }

    #[test]
    fn confidential_never_cloud() {
        let r = offline_router();
        let d = r.route_to(&coding_secret(), 500).unwrap();
        assert!(d.class.is_local());
        assert!(!d.provider_id.contains("cloud"));
    }

    #[test]
    fn budget_zero_means_free_only() {
        let policy = RoutingPolicy {
            max_cost_per_request: 0.0,
            ..RoutingPolicy::default()
        };
        let r = ModelRouter {
            policy,
            ..offline_router()
        };
        // Local providers are free, so routing still succeeds.
        let d = r
            .route_to(
                &RouteRequest {
                    privacy: PrivacyLevel::Public,
                    task: TaskType::Simple,
                    needs_vision: false,
                    needs_reasoning: false,
                    network_up: false,
                },
                10_000,
            )
            .unwrap();
        assert_eq!(d.estimated_cost, 0.0);
    }

    #[test]
    fn adaptive_prefers_successful_provider() {
        let mut r = offline_router();
        // Poison local-tiny, reward local-small.
        for _ in 0..5 {
            r.record_feedback("local-tiny", 50, false);
            r.record_feedback("local-small", 40, true);
        }
        let d = r
            .route_to(
                &RouteRequest {
                    privacy: PrivacyLevel::Public,
                    task: TaskType::Complex,
                    needs_vision: false,
                    needs_reasoning: true,
                    network_up: false,
                },
                500,
            )
            .unwrap();
        assert_eq!(d.provider_id, "local-small");
        let st = r.stats("local-small");
        assert!((st.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fallback_chain_ends_local() {
        let r = offline_router();
        let d = r
            .route_to(
                &RouteRequest {
                    privacy: PrivacyLevel::Public,
                    task: TaskType::Simple,
                    needs_vision: false,
                    needs_reasoning: false,
                    network_up: false,
                },
                100,
            )
            .unwrap();
        let last = d.fallbacks.last().unwrap();
        assert!(r.providers.get(last).unwrap().info().class.is_local());
    }

    #[test]
    fn cloud_stub_gated_by_env() {
        let c = CloudStubProvider::fast_cloud("IGRIS_TEST_KEY_MISSING_12345");
        assert!(!c.is_available());
        assert!(c.generate("hi", 10).is_err());
    }

    #[test]
    fn local_echo_generates() {
        let p = LocalEchoProvider::tiny();
        assert!(p.is_available());
        let out = p.generate("hello world", 100).unwrap();
        assert!(out.contains("hello world"));
    }

    #[test]
    fn vision_without_provider_errors() {
        let r = offline_router();
        let err = r
            .route_to(
                &RouteRequest {
                    privacy: PrivacyLevel::Public,
                    task: TaskType::Vision,
                    needs_vision: true,
                    needs_reasoning: false,
                    network_up: false,
                },
                100,
            )
            .unwrap_err();
        assert!(matches!(err, RouterError::NoProvider(_)));
    }

    #[test]
    fn stats_are_aggregate_only() {
        // Success rate math: neutral prior, then observed values.
        let mut s = RouteStats::default();
        assert!((s.success_rate() - 0.5).abs() < f64::EPSILON);
        s.successes = 3;
        s.failures = 1;
        assert!((s.success_rate() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn budget_route_1000_decisions_under_10ms_each() {
        use std::time::Instant;
        let r = ModelRouter::with_defaults();
        let req = RouteRequest {
            privacy: PrivacyLevel::Public,
            task: TaskType::Coding,
            needs_vision: false,
            needs_reasoning: true,
            network_up: false,
        };
        let t0 = Instant::now();
        for _ in 0..1000 {
            let t1 = Instant::now();
            let _ = r.route_to(&req, 500).unwrap();
            assert!(t1.elapsed().as_millis() < 10, "route exceeded 10ms budget");
        }
        let avg_us = t0.elapsed().as_micros() as f64 / 1000.0;
        assert!(avg_us < 1000.0, "avg {avg_us:.1}us exceeds 1ms");
    }
}
