# MODELS.md — IGRIS v5 model manifest (provisioned 2026-09-20)

Every entry: source URL, size, license, status, consumer. Anything marked
QUEUED has its integration seam ready but unwired — no silent placeholders.

## Provisioned (downloaded, verified working)

### Piper TTS engine — `models/piper/bin/piper/`
- Source: https://github.com/rhasspy/piper/releases/tag/2023.11.14-2 (`piper_windows_amd64.zip`, 22MB)
- Contents: `piper.exe` + `espeak-ng.dll` + phoneme data + onnxruntime DLLs
- License: MIT (engine). Voices carry their own terms (see below).
- Status: ✅ LIVE — synthesized `first-run.wav` (3.0s audio, 0.2s infer, 15x realtime)
- Consumer: `PiperTts` (`igris-voice/src/tts.rs`) via `PiperTts::bundled()`

### English voice, Lessac medium — `models/piper/voices/`
- Source: https://huggingface.co/rhasspy/piper-voices (`en/en_US/lessac/medium/`, 63MB `.onnx` + `.onnx.json`)
- License: check voice README on HuggingFace before redistribution (local use fine).
- Status: ✅ LIVE (default voice for `PiperTts::bundled()`)

### SenseVoice multilingual STT (int8) — `models/stt/sense-voice/`
- Source: https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17 (`model.int8.onnx` 239MB + `tokens.txt`)
- License: Apache-2.0 (model weights follow FunAudioLLM terms; local use fine).
- Status: ⏳ ON DISK, integration QUEUED — needs the `sherpa-onnx` crate adapter behind `SpeechToText` (planned `onnx` feature on `igris-voice`).

### MiniLM embeddings — `models/embed/minilm/`
- Source: https://huggingface.co/Xenova/all-MiniLM-L6-v2 (`onnx/model.onnx` 90MB + `tokenizer.json`)
- License: Apache-2.0.
- Status: ⏳ ON DISK, integration QUEUED — needs an `ort` adapter behind `EmbeddingProvider` (replaces the hash stub).

## Queued (documented, not downloaded — no runner integrated yet)

| Need | Candidate | Approx size | Note |
|---|---|---|---|
| Local reasoning LLM | DeepSeek-R1-Distill-Qwen-14B (Q4) or Qwen2.5-14B via llama.cpp | 8–10GB | No llama runner integrated; router `LocalSmall` class reserved. Decide before downloading. |
| Acoustic wake model | openWakeWord `hey_jarvis` ONNX (~15MB) | 15MB | Would replace fuzzy text-matcher; needs ort VAD pipeline first. |
| OS TTS fallback | Windows SAPI / WinRT SpeechSynthesis | 0 (OS) | Needs WinRT async wiring; Piper covers offline TTS today. |

## Rules

1. Nothing under `models/` is committed (`.gitignore`: `models/*` except docs).
2. New models land here with a MODELS.md entry: URL, size, license, consumer.
3. Licensed voices/models are local-use; check redistribution terms before shipping installers.
4. Re-provision: follow the URLs above; verify with the commands in `models/README.md`.
