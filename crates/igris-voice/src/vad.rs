//! Frame-level voice activity detection: RMS energy gate with a
//! zero-crossing sanity band. Pure Rust, no models — the acoustic heavy
//! lifting (neural VAD/STT) plugs in behind the `SpeechToText` trait later.

/// 20ms of mono f32 audio. 16kHz is the pipeline standard.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioFrame {
    pub fn silence(frames: usize, sample_rate: u32) -> Vec<Self> {
        vec![
            Self {
                samples: vec![0.0; frames],
                sample_rate
            };
            1
        ]
    }

    /// Synthetic tone for tests: `freq` Hz sine at `amplitude`.
    pub fn tone(freq: f32, amplitude: f32, frames: usize, sample_rate: u32) -> Self {
        let samples = (0..frames)
            .map(|i| {
                amplitude
                    * (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin()
            })
            .collect();
        Self {
            samples,
            sample_rate,
        }
    }

    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.samples.iter().map(|s| s * s).sum::<f32>() / self.samples.len() as f32).sqrt()
    }

    /// Fraction of sign changes between consecutive samples.
    pub fn zero_crossing_rate(&self) -> f32 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let mut crossings = 0;
        for w in self.samples.windows(2) {
            if (w[0] >= 0.0) != (w[1] >= 0.0) {
                crossings += 1;
            }
        }
        crossings as f32 / (self.samples.len() - 1) as f32
    }
}

#[derive(Debug, Clone)]
pub struct VadConfig {
    /// RMS below this is silence.
    pub energy_threshold: f32,
    /// ZCR outside this band is not speech (rumble/hiss rejection).
    pub zcr_min: f32,
    pub zcr_max: f32,
    /// Silent frames before a segment closes (hangover).
    pub hangover_frames: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.02,
            zcr_min: 0.005,
            zcr_max: 0.6,
            hangover_frames: 15,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Idle,
    Speaking,
}

#[derive(Debug)]
pub struct Vad {
    config: VadConfig,
    state: VadState,
    silent_run: usize,
    segment: Vec<f32>,
}

impl Vad {
    pub fn new(config: VadConfig) -> Self {
        Self {
            config,
            state: VadState::Idle,
            silent_run: 0,
            segment: Vec::new(),
        }
    }

    pub fn state(&self) -> VadState {
        self.state
    }

    pub fn is_speech_frame(&self, frame: &AudioFrame) -> bool {
        if frame.rms() < self.config.energy_threshold {
            return false;
        }
        let zcr = frame.zero_crossing_rate();
        zcr >= self.config.zcr_min && zcr <= self.config.zcr_max
    }

    /// Feed one frame. Returns the completed segment (raw samples) when
    /// speech ends, or `None` while listening / in silence.
    pub fn ingest(&mut self, frame: &AudioFrame) -> Option<Vec<f32>> {
        if self.is_speech_frame(frame) {
            self.state = VadState::Speaking;
            self.silent_run = 0;
            self.segment.extend_from_slice(&frame.samples);
            None
        } else if self.state == VadState::Speaking {
            self.silent_run += 1;
            if self.silent_run >= self.config.hangover_frames {
                self.state = VadState::Idle;
                self.silent_run = 0;
                Some(std::mem::take(&mut self.segment))
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;
    const FRAME: usize = 320; // 20ms @ 16kHz

    #[test]
    fn silence_is_not_speech() {
        let vad = Vad::new(VadConfig::default());
        let f = AudioFrame {
            samples: vec![0.0; FRAME],
            sample_rate: SR,
        };
        assert!(!vad.is_speech_frame(&f));
    }

    #[test]
    fn tone_is_speech_low_noise_is_not() {
        let vad = Vad::new(VadConfig::default());
        let speech = AudioFrame::tone(440.0, 0.5, FRAME, SR);
        assert!(vad.is_speech_frame(&speech));
        let noise = AudioFrame::tone(440.0, 0.005, FRAME, SR);
        assert!(!vad.is_speech_frame(&noise));
    }

    #[test]
    fn hangover_closes_segment() {
        let mut vad = Vad::new(VadConfig {
            hangover_frames: 3,
            ..Default::default()
        });
        let speech = AudioFrame::tone(440.0, 0.5, FRAME, SR);
        let quiet = AudioFrame {
            samples: vec![0.0; FRAME],
            sample_rate: SR,
        };
        assert!(vad.ingest(&speech).is_none());
        assert!(vad.ingest(&speech).is_none());
        assert!(vad.ingest(&quiet).is_none());
        assert!(vad.ingest(&quiet).is_none());
        let seg = vad.ingest(&quiet).expect("segment closes after hangover");
        assert_eq!(seg.len(), 2 * FRAME);
        assert_eq!(vad.state(), VadState::Idle);
        // Pure silence never opens a segment.
        assert!(vad.ingest(&quiet).is_none());
    }
}
