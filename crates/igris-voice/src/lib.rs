//! igris-voice — local-first voice loop: VAD, wake word, reflex routing.
//!
//! Phase 10a: real segmentation (energy + zero-crossing VAD), fuzzy wake
//! matching, trait-seamed STT/TTS (neural models plug in later), and a
//! pipeline where barge-in cancels synthesis before it wastes another frame.
//! The reflex fast path never invokes an LLM for simple voice commands.

pub mod capture;
pub mod pipeline;
pub mod tts;
pub mod vad;
pub mod wake;

pub use capture::{kennedy, list_mics, CaptureError, MicInfo};
pub use pipeline::{
    RecordingTts, ScriptStt, SpeechToText, TextToSpeech, VoiceError, VoiceEvent, VoicePipeline,
    VoiceState,
};
pub use tts::{auto_detect, PiperTts, TtsError, VoiceBackendStatus};
pub use vad::{AudioFrame, Vad, VadConfig, VadState};
pub use wake::WakeMatcher;
