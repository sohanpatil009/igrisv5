//! Voice pipeline: VAD segments -> STT -> wake check -> reflex fast path.
//!
//! The expensive pieces (neural STT, neural TTS) sit behind traits with
//! honest stubs. What IS real today: segmentation, wake gating, reflex
//! routing without an LLM on fast paths, and barge-in — user speech
//! cancels in-progress synthesis before it wastes another frame.

use crate::vad::{AudioFrame, Vad, VadConfig};
use crate::wake::WakeMatcher;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum VoiceError {
    #[error("transcription unavailable: {0}")]
    SttUnavailable(String),
    #[error("synthesis unavailable: {0}")]
    TtsUnavailable(String),
    #[error("empty segment")]
    EmptySegment,
}

/// Transcript source. Neural models implement this later; tests use ScriptStt.
pub trait SpeechToText: Send {
    fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, VoiceError>;
}

/// Speech sink with cancellation for barge-in.
pub trait TextToSpeech: Send {
    fn speak(&mut self, text: &str);
    fn cancel(&mut self);
    fn speaking(&self) -> bool;
    fn spoken(&self) -> &[String];
}

/// Deterministic STT for tests and demos: yields queued transcripts.
#[derive(Debug, Default)]
pub struct ScriptStt {
    queue: VecDeque<String>,
    pub calls: usize,
}

impl ScriptStt {
    pub fn new(transcripts: &[&str]) -> Self {
        Self {
            queue: transcripts.iter().map(|s| s.to_string()).collect(),
            calls: 0,
        }
    }
}

impl SpeechToText for ScriptStt {
    fn transcribe(&mut self, samples: &[f32], _sample_rate: u32) -> Result<String, VoiceError> {
        self.calls += 1;
        if samples.is_empty() {
            return Err(VoiceError::EmptySegment);
        }
        self.queue
            .pop_front()
            .ok_or_else(|| VoiceError::SttUnavailable("script exhausted".into()))
    }
}

/// Recording TTS: stores utterances, honours cancel. No audio hardware.
#[derive(Debug, Default)]
pub struct RecordingTts {
    utterances: Vec<String>,
    cancelled: usize,
    live: bool,
}

impl TextToSpeech for RecordingTts {
    fn speak(&mut self, text: &str) {
        self.utterances.push(text.into());
        self.live = true;
    }

    fn cancel(&mut self) {
        if self.live {
            self.cancelled += 1;
            self.live = false;
        }
    }

    fn speaking(&self) -> bool {
        self.live
    }

    fn spoken(&self) -> &[String] {
        &self.utterances
    }
}

impl RecordingTts {
    pub fn cancel_count(&self) -> usize {
        self.cancelled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Speaking,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    SpeechStarted,
    WakeDetected { transcript: String },
    IgnoredNoWake,
    FastPath { intent: String },
    Escalated { intent: String },
    SpeakingStarted { text: String },
    SpeakingCancelled,
    SttFailed,
}

/// Short spoken acks for fast-path intents. Full responses need the
/// reasoning pipeline; the ack buys time without an LLM call.
fn fast_ack(intent: &str) -> Option<&'static str> {
    match intent {
        "device_transfer" => Some("On it — sending."),
        "device_status" => Some("Checking now."),
        "memory_query" => Some("Recalling."),
        _ => None,
    }
}

pub struct VoicePipeline<S: SpeechToText, T: TextToSpeech> {
    vad: Vad,
    wake: WakeMatcher,
    stt: S,
    tts: T,
    state: VoiceState,
    /// Shared with audio output: set while synthesis is live so fresh
    /// speech frames can pre-empt it (barge-in).
    barge_in: Arc<AtomicBool>,
}

impl<S: SpeechToText, T: TextToSpeech> VoicePipeline<S, T> {
    pub fn new(stt: S, tts: T) -> Self {
        Self {
            vad: Vad::new(VadConfig::default()),
            wake: WakeMatcher::arise(),
            stt,
            tts,
            state: VoiceState::Idle,
            barge_in: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn state(&self) -> VoiceState {
        self.state
    }

    pub fn barge_flag(&self) -> Arc<AtomicBool> {
        self.barge_in.clone()
    }

    /// External barge-in: user started talking over synthesis.
    pub fn interrupt(&mut self) -> bool {
        if self.tts.speaking() {
            self.tts.cancel();
            self.barge_in.store(false, Ordering::SeqCst);
            self.state = VoiceState::Listening;
            true
        } else {
            false
        }
    }

    /// Feed one audio frame. Drives VAD -> segment -> STT -> reflex.
    /// While synthesis is live, fresh speech frames pre-empt it first.
    pub fn ingest(&mut self, frame: &AudioFrame) -> Vec<VoiceEvent> {
        // Barge-in path: live speech over live synthesis wins immediately.
        if self.tts.speaking() && self.vad.is_speech_frame(frame) {
            self.tts.cancel();
            self.barge_in.store(false, Ordering::SeqCst);
            self.state = VoiceState::Listening;
            return vec![VoiceEvent::SpeakingCancelled];
        }
        let mut events = Vec::new();
        let was_speaking = self.vad.state() == crate::vad::VadState::Speaking;
        match self.vad.ingest(frame) {
            None => {
                if !was_speaking && self.vad.state() == crate::vad::VadState::Speaking {
                    self.state = VoiceState::Listening;
                    events.push(VoiceEvent::SpeechStarted);
                }
            }
            Some(segment) => {
                self.state = VoiceState::Processing;
                match self.stt.transcribe(&segment, frame.sample_rate) {
                    Err(_) => {
                        self.state = VoiceState::Error;
                        events.push(VoiceEvent::SttFailed);
                    }
                    Ok(transcript) => {
                        if !self.wake.matches(&transcript) {
                            self.state = VoiceState::Idle;
                            events.push(VoiceEvent::IgnoredNoWake);
                            return events;
                        }
                        events.push(VoiceEvent::WakeDetected {
                            transcript: transcript.clone(),
                        });
                        // Reflex fast path: no LLM here, ever.
                        let d = igris_reflex::decide(&transcript);
                        let intent = intent_to_key(d.intent).to_string();
                        if d.requires_reasoning {
                            self.state = VoiceState::Idle;
                            events.push(VoiceEvent::Escalated { intent });
                        } else {
                            events.push(VoiceEvent::FastPath {
                                intent: intent.clone(),
                            });
                            if let Some(ack) = fast_ack(&intent) {
                                self.tts.speak(ack);
                                self.barge_in.store(true, Ordering::SeqCst);
                                self.state = VoiceState::Speaking;
                                events.push(VoiceEvent::SpeakingStarted { text: ack.into() });
                            } else {
                                self.state = VoiceState::Idle;
                            }
                        }
                    }
                }
            }
        }
        events
    }
}

fn intent_to_key(intent: igris_reflex::Intent) -> &'static str {
    match intent {
        igris_reflex::Intent::DeviceTransfer => "device_transfer",
        igris_reflex::Intent::DeviceStatus => "device_status",
        igris_reflex::Intent::MemoryQuery => "memory_query",
        igris_reflex::Intent::ToolExec => "tool_exec",
        igris_reflex::Intent::Communication => "communication",
        igris_reflex::Intent::RiskyOp => "risky_op",
        igris_reflex::Intent::General => "general",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;
    const FRAME: usize = 320;

    fn speech_frames(n: usize) -> Vec<AudioFrame> {
        (0..n)
            .map(|_| AudioFrame::tone(440.0, 0.5, FRAME, SR))
            .collect()
    }

    fn quiet_frames(n: usize) -> Vec<AudioFrame> {
        (0..n)
            .map(|_| AudioFrame {
                samples: vec![0.0; FRAME],
                sample_rate: SR,
            })
            .collect()
    }

    fn drive<S: SpeechToText, T: TextToSpeech>(
        p: &mut VoicePipeline<S, T>,
        frames: Vec<AudioFrame>,
    ) -> Vec<VoiceEvent> {
        let mut out = Vec::new();
        for f in &frames {
            out.extend(p.ingest(f));
        }
        out
    }

    #[test]
    fn full_fast_path_loop() {
        let mut p = VoicePipeline::new(
            ScriptStt::new(&["arise send the build to my laptop"]),
            RecordingTts::default(),
        );
        let mut frames = speech_frames(4);
        frames.extend(quiet_frames(20));
        let events = drive(&mut p, frames);
        assert!(events.contains(&VoiceEvent::SpeechStarted));
        assert!(events
            .iter()
            .any(|e| matches!(e, VoiceEvent::WakeDetected { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, VoiceEvent::FastPath { intent } if intent == "device_transfer")));
        assert!(events
            .iter()
            .any(|e| matches!(e, VoiceEvent::SpeakingStarted { .. })));
        assert_eq!(p.state(), VoiceState::Speaking);
    }

    #[test]
    fn no_wake_no_action() {
        let mut p = VoicePipeline::new(
            ScriptStt::new(&["what time is it"]),
            RecordingTts::default(),
        );
        let mut frames = speech_frames(4);
        frames.extend(quiet_frames(20));
        let events = drive(&mut p, frames);
        assert!(events.contains(&VoiceEvent::IgnoredNoWake));
        assert!(!events
            .iter()
            .any(|e| matches!(e, VoiceEvent::SpeakingStarted { .. })));
    }

    #[test]
    fn risky_voice_command_escalates_without_speaking() {
        let mut p = VoicePipeline::new(
            ScriptStt::new(&["arise delete everything"]),
            RecordingTts::default(),
        );
        let mut frames = speech_frames(4);
        frames.extend(quiet_frames(20));
        let events = drive(&mut p, frames);
        assert!(events
            .iter()
            .any(|e| matches!(e, VoiceEvent::Escalated { .. })));
        assert!(!events
            .iter()
            .any(|e| matches!(e, VoiceEvent::SpeakingStarted { .. })));
    }

    #[test]
    fn barge_in_cancels_synthesis() {
        let mut p = VoicePipeline::new(
            ScriptStt::new(&["arise send the build to my laptop", "arise stop that"]),
            RecordingTts::default(),
        );
        let mut frames = speech_frames(4);
        frames.extend(quiet_frames(20));
        drive(&mut p, frames);
        assert_eq!(p.state(), VoiceState::Speaking);
        // Fresh speech while synthesis is live pre-empts it.
        let events = p.ingest(&AudioFrame::tone(440.0, 0.5, FRAME, SR));
        assert!(events.contains(&VoiceEvent::SpeakingCancelled));
        assert_eq!(p.state(), VoiceState::Listening);
    }

    #[test]
    fn explicit_interrupt_is_safe_when_idle() {
        let mut p = VoicePipeline::new(ScriptStt::new(&[]), RecordingTts::default());
        assert!(!p.interrupt());
    }

    #[test]
    fn stt_failure_surfaces_error_state() {
        let mut p = VoicePipeline::new(ScriptStt::new(&[]), RecordingTts::default());
        let mut frames = speech_frames(4);
        frames.extend(quiet_frames(20));
        let events = drive(&mut p, frames);
        assert!(events.contains(&VoiceEvent::SttFailed));
        assert_eq!(p.state(), VoiceState::Error);
    }
}
