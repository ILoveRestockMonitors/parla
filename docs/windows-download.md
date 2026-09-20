# Install Parla for Windows

**[Download Parla Setup.exe](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-Setup.exe)**

The Windows x64 installer includes Parla, Parakeet and Whisper, both speech models, and a private Python runtime. Basic dictation requires no separate downloads, Python installation, GPU software, Ollama, or account. Installation and Faithful dictation can run offline after downloading the installer.

## Install and dictate

1. Run `Parla-Setup.exe` and complete the wizard. It installs for your Windows account and offers desktop and Start menu shortcuts.
2. Leave **Open Parla** selected on the final page, or use either shortcut. Parla starts in the background and opens its dashboard in your browser.
3. Wait for **Speech server** to show **live**. The default is Parakeet with Faithful cleanup; initial model loading can take a few seconds.
4. Click a text field, press **Ctrl+Space**, speak, then press **Ctrl+Space** again. You can also hold **Ctrl+Win** while speaking. Permit microphone access for desktop apps in Windows if necessary.

The installer places about 1.3 GB of files under `%LOCALAPPDATA%\Programs\Parla`. Recognition runs on the CPU. Speed and accuracy depend on your computer and speech. Whisper is also included and can be selected from the dashboard.

The app and installer are unsigned, so Windows may show an unknown-publisher prompt. Use the linked GitHub release; its `SHA256SUMS.txt` records download checksums. Do not disable Windows security protections.

## What is included

| Component | Included |
| --- | --- |
| Parla native app and dashboard | Yes |
| Parakeet TDT 0.6B v2 INT8 model | Yes; default speech model |
| Python 3.12.10, sherpa-onnx/core 1.13.7, NumPy 2.4.6 | Yes; private to Parla |
| whisper.cpp v1.8.7 CPU server and Whisper small model | Yes; alternative speech engine |
| Ollama and a polishing model | Optional; separate installation |

The models are carried inside `Setup.exe` and extracted into Parla's installation folder. The app loads them from disk; they are not downloaded on first launch. Private Python does not change system Python or PATH. The Whisper server includes its compiler runtimes and does not require a separate Visual C++ installation.

## Optional polishing with Ollama

**Faithful** works without Ollama. For **Polished** cleanup, install [Ollama for Windows](https://ollama.com/download/windows), open it, and download the exact model named in Parla's **Setup & downloads** panel. Its model-library link helps locate that model. Select Polished in Settings when ready.

The setup panel checks whether Ollama is responding and whether its configured model is downloaded. A stopped service is not treated as proof that the model is missing. Polishing failures preserve the recognized wording and report a warning.

## Updates and personal data

Finish active dictation and close Parla before updating the same installation folder. The installer does not force-stop the running app. Existing settings are preserved, including deliberately configured external speech engines/models. Included engines are selected automatically for a fresh installation.

Settings, dictionary, history, and runtime logs remain under `%LOCALAPPDATA%\Parla`. Default history expires after 24 hours. Uninstall from Windows **Installed apps**. Uninstallation removes the app and bundled models while retaining personal data. Reinstalling can reuse those settings.

## Troubleshooting and manual installations

**Setup & downloads** identifies missing configured files and provides official download/setup links. Recheck after changes. File presence does not prove that a model has loaded; use Speech server status for readiness. Restart after changing model paths or installing an external Python environment.

For an advanced manual Whisper installation, obtain [whisper.cpp](https://github.com/ggml-org/whisper.cpp/releases) and [ggml-small.bin](https://huggingface.co/ggerganov/whisper.cpp), then use `scripts/initialize-settings.ps1` with their actual full paths. This is unnecessary for a fresh bundled installation.

### Optional Parakeet setup

The installer already contains Parakeet. For an advanced manual installation only, use [Python for Windows](https://www.python.org/downloads/windows/) with Python 3.12 x64 and the matching [Parakeet TDT 0.6B v2 INT8 model](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html).

```powershell
py -3.12 -m venv "$env:LOCALAPPDATA\Parla\venv"
& "$env:LOCALAPPDATA\Parla\venv\Scripts\python.exe" -m pip install 'sherpa-onnx==1.13.7' 'numpy==2.4.6'
$env:PARLA_PARAKEET_PYTHON = "$env:LOCALAPPDATA\Parla\venv\Scripts\python.exe"
```

Put matching `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`, and `tokens.txt` directly in the configured `parakeet_model_dir`. Launch Parla from that shell to use the explicit interpreter override. Installer builds otherwise prefer their private interpreter. The historical guide's `PYTHON` variable is superseded by `PARLA_PARAKEET_PYTHON`.

## Verification and provenance

The release's `bundle-manifest.json` records installed-file hashes and pinned download sources. Model attribution and third-party notices are installed alongside the app. See [current state](current-state.md) and the [changelog](../CHANGELOG.md) for verification and known limitations.
