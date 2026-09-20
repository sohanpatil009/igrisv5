//! igris-vision — multimodal on a budget.
//!
//! Vision models fire only when the gate says so: an explicit image ask or
//! a reflex decision that needs vision. Everything else stays text-only so
//! context and cost never pay the image tax by accident.

use serde::{Deserialize, Serialize};

/// What the caller wants eyes on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionRequest {
    /// Opaque handle (path, capture id, screen region). Never the pixels —
    /// the describer resolves the handle under policy.
    pub source: String,
    pub question: String,
    pub max_detail: Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Detail {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContext {
    pub description: String,
    pub detail: Detail,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum VisionError {
    #[error("no vision provider wired")]
    NotWired,
    #[error("gate refused: {0}")]
    GateRefused(String),
}

/// Decides whether pixels are worth it.
#[derive(Debug, Clone, Default)]
pub struct VisionGate;

impl VisionGate {
    pub fn new() -> Self {
        Self
    }

    /// Invoke vision iff the user attached/asked for an image, or the reflex
    /// decision explicitly needs vision. Text-only traffic never passes.
    pub fn should_invoke(
        &self,
        has_image: bool,
        reflex_needs_vision: bool,
        explicit_ask: bool,
    ) -> bool {
        has_image && (reflex_needs_vision || explicit_ask)
    }
}

/// Vision backend seam. Real multimodal models implement this later.
pub trait ImageDescriber: Send {
    fn describe(&self, req: &VisionRequest) -> Result<ImageContext, VisionError>;
}

/// Honest placeholder: always refuses so callers handle the unwired path.
pub struct NullDescriber;

impl ImageDescriber for NullDescriber {
    fn describe(&self, _req: &VisionRequest) -> Result<ImageContext, VisionError> {
        Err(VisionError::NotWired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_only_fires_with_image_and_reason() {
        let g = VisionGate::new();
        assert!(!g.should_invoke(false, true, true), "no pixels, no invoke");
        assert!(
            !g.should_invoke(true, false, false),
            "image without reason stays text"
        );
        assert!(g.should_invoke(true, true, false));
        assert!(g.should_invoke(true, false, true));
    }

    #[test]
    fn null_describer_refuses_honestly() {
        let d = NullDescriber;
        let req = VisionRequest {
            source: "screen://main".into(),
            question: "what is open?".into(),
            max_detail: Detail::Low,
        };
        assert!(matches!(d.describe(&req), Err(VisionError::NotWired)));
    }

    #[test]
    fn request_serializes_for_transport() {
        let req = VisionRequest {
            source: "cam://0".into(),
            question: "who is there?".into(),
            max_detail: Detail::High,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: VisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source, "cam://0");
    }
}
