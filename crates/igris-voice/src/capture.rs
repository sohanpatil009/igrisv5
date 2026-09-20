//! Microphone capture over cpal (WASAPI/CoreAudio/ALSA per platform).
//! Device enumeration never fails the process — no hardware simply yields
//! an empty list, and recording without hardware errors honestly instead
//! of hanging. Samples are normalized to mono f32 for the VAD/pipeline.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Debug, Clone, thiserror::Error)]
pub enum CaptureError {
    #[error("no input device: {0}")]
    NoDevice(String),
    #[error("unsupported stream: {0}")]
    Unsupported(String),
    #[error("capture failed: {0}")]
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct MicInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// List input devices. Empty (not an error) when headless.
pub fn list_mics() -> Vec<MicInfo> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    let Ok(devices) = host.input_devices() else {
        return out;
    };
    for d in devices {
        let name = d.name().unwrap_or_else(|_| "<unknown>".into());
        let (sample_rate, channels) = d
            .default_input_config()
            .map(|c| (c.sample_rate().0, c.channels()))
            .unwrap_or((0, 0));
        out.push(MicInfo {
            name,
            sample_rate,
            channels,
        });
    }
    out
}

/// Blocking recorder (separate namespace so tests read clean).
pub mod kennedy {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Samples {
        pub data: Vec<f32>,
        pub sample_rate: u32,
    }

    /// Blocking record. Runs on a worker thread in async contexts.
    pub fn record_blocking(millis: u64, device_index: usize) -> Result<Samples, CaptureError> {
        let host = cpal::default_host();
        let mut devices = host
            .input_devices()
            .map_err(|e| CaptureError::NoDevice(e.to_string()))?;
        let device = devices
            .nth(device_index)
            .ok_or_else(|| CaptureError::NoDevice(format!("no mic at index {device_index}")))?;
        let config = device
            .default_input_config()
            .map_err(|e| CaptureError::Unsupported(e.to_string()))?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    let _ = tx.send(data.to_vec());
                },
                |e| eprintln!("mic stream error: {e}"),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _| {
                    let _ = tx.send(data.iter().map(|s| *s as f32 / 32768.0).collect());
                },
                |e| eprintln!("mic stream error: {e}"),
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |data: &[u16], _| {
                    let _ = tx.send(
                        data.iter()
                            .map(|s| (*s as f32 - 32768.0) / 32768.0)
                            .collect(),
                    );
                },
                |e| eprintln!("mic stream error: {e}"),
                None,
            ),
            f => return Err(CaptureError::Unsupported(format!("{f:?}"))),
        }
        .map_err(|e| CaptureError::Failed(e.to_string()))?;

        stream
            .play()
            .map_err(|e| CaptureError::Failed(e.to_string()))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        let mut mono = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(chunk) => {
                    // Downmix to mono.
                    for frame in chunk.chunks(channels.max(1)) {
                        let sum: f32 = frame.iter().sum();
                        mono.push(sum / frame.len().max(1) as f32);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(Samples {
            data: mono,
            sample_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_never_crashes_headless() {
        // May be empty in CI/sandbox — the property is "no panic, no error".
        let _mics: Vec<MicInfo> = list_mics();
    }

    #[test]
    fn record_without_hardware_errors_honestly() {
        // Index 9999 cannot exist; must be a typed NoDevice, never a hang.
        let err = kennedy::record_blocking(50, 9999).unwrap_err();
        assert!(matches!(err, CaptureError::NoDevice(_)));
    }

    #[test]
    fn record_if_hardware_present() {
        if list_mics().is_empty() {
            return; // headless CI: enumeration test above already covers us
        }
        let s = kennedy::record_blocking(150, 0).expect("mic records");
        assert!(s.sample_rate > 0);
        assert!(!s.data.is_empty(), "even a muted mic delivers frames");
    }
}
