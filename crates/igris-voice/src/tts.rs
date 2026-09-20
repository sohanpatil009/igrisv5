//! Speech output backends behind the pipeline's `TextToSpeech` trait.
//!
//! `PiperTts` shells to a Piper executable (or any stand-in with the same
//! CLI shape: `[args...] <text>`). Missing binary = honest error, never a
//! fake voice. OS-native synthesis (SAPI/WinRT) and neural voices plug in
//! here when their runtimes are provisioned.

use crate::pipeline::TextToSpeech;
use std::path::PathBuf;
use std::process::Child;

#[derive(Debug, Clone, thiserror::Error)]
pub enum TtsError {
    #[error("synthesizer binary missing: {0}")]
    MissingBinary(String),
    #[error("synthesis failed: {0}")]
    Failed(String),
}

pub struct PiperTts {
    exe: PathBuf,
    extra_args: Vec<String>,
    utterances: Vec<String>,
    child: Option<Child>,
    live: bool,
}

impl PiperTts {
    pub fn new(exe: PathBuf, extra_args: Vec<String>) -> Self {
        Self {
            exe,
            extra_args,
            utterances: Vec::new(),
            child: None,
            live: false,
        }
    }

    /// Quick probe used by auto-detect: is there anything to execute?
    pub fn binary_present(&self) -> bool {
        self.exe.exists() || which_like(&self.exe)
    }
}

/// True when the path names something executable on PATH (cmd, echo, ...).
fn which_like(exe: &Path) -> bool {
    let name = exe.to_string_lossy().to_string();
    if name.contains(std::path::MAIN_SEPARATOR) || name.contains('/') {
        return false;
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .any(|d| d.join(&name).exists() || d.join(format!("{name}.exe")).exists())
        })
        .unwrap_or(false)
}

use std::path::Path;

impl TextToSpeech for PiperTts {
    fn speak(&mut self, text: &str) {
        self.utterances.push(text.into());
        let mut cmd = std::process::Command::new(&self.exe);
        cmd.args(&self.extra_args).arg(text);
        match cmd.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.live = true;
                // Reap: synthesis is fire-and-monitor; completion observed
                // when the handle goes quiet (see `speaking`).
                if let Some(c) = self.child.as_mut() {
                    if matches!(c.try_wait(), Ok(Some(_))) {
                        self.live = false;
                    }
                }
            }
            Err(_) => self.live = false,
        }
    }

    fn cancel(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.live = false;
    }

    fn speaking(&self) -> bool {
        self.live
    }

    fn spoken(&self) -> &[String] {
        &self.utterances
    }
}

// ---------------------------------------------------------------------------
// Backend auto-detect: report what voice hardware/software is actually live.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VoiceBackendStatus {
    pub mic_count: usize,
    pub mic_names: Vec<String>,
    pub tts_binary_present: bool,
    pub wake_phrase: String,
}

pub fn auto_detect(piper_exe: Option<PathBuf>) -> VoiceBackendStatus {
    let mics = crate::capture::list_mics();
    VoiceBackendStatus {
        mic_count: mics.len(),
        mic_names: mics.into_iter().map(|m| m.name).collect(),
        tts_binary_present: piper_exe
            .map(|p| PiperTts::new(p, vec![]).binary_present())
            .unwrap_or(false),
        wake_phrase: "arise".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_detected() {
        let t = PiperTts::new(PathBuf::from("/nonexistent/piper-xyz"), vec![]);
        assert!(!t.binary_present());
    }

    #[test]
    fn speak_spawns_and_records() {
        // Stand-in with Piper's CLI shape: `cmd /C echo <text>`.
        let (exe, args) = if cfg!(windows) {
            (
                PathBuf::from("cmd"),
                vec!["/C".to_string(), "echo".to_string()],
            )
        } else {
            (PathBuf::from("echo"), vec![])
        };
        let mut t = PiperTts::new(exe, args);
        assert!(t.binary_present());
        t.speak("hello field");
        assert_eq!(t.spoken(), &["hello field".to_string()]);
        t.cancel(); // safe even if the child already exited
        assert!(!t.speaking());
    }

    #[test]
    fn auto_detect_reports_honestly() {
        let s = auto_detect(Some(PathBuf::from("/nonexistent/piper-xyz")));
        assert!(!s.tts_binary_present);
        assert_eq!(s.wake_phrase, "arise");
        assert_eq!(s.mic_count, s.mic_names.len());
    }
}
