# models/ — local model + engine binaries (git-ignored, re-downloadable)

Large files in this directory are **never committed**. This folder documents
what lives here and where to get it. Provision a fresh machine with the
commands in `MODELS.md`.

Layout:

```text
models/
├── MODELS.md            # full manifest (this file's big sibling) — committed
├── README.md            # quickstart — committed
├── piper/
│   ├── bin/piper/       # Piper 2023.11.14-2 engine (piper.exe + espeak-ng + onnxruntime)
│   ├── voices/en_US-lessac-medium.onnx(.json)
│   └── first-run.wav    # proof synthesis (regenerable)
├── stt/sense-voice/     # SenseVoice int8 ONNX + tokens (sherpa integration queued)
└── embed/minilm/        # all-MiniLM-L6-v2 ONNX + tokenizer (ort integration queued)
```

## Quickstart (Windows)

```powershell
# Speak through the provisioned voice (CPU, ~15x realtime):
"Igris field online." | .\models\piper\bin\piper\piper.exe `
  --model .\models\piper\voices\en_US-lessac-medium.onnx `
  --output_file out.wav
```

`PiperTts::bundled()` in `igris-voice` resolves these paths automatically
when the process runs from the workspace root.
